//! Tauri IPC commands. All file IO happens here in Rust, so no filesystem plugin scope
//! has to be configured for arbitrary user-selected log files.

use crate::adb;
use crate::builtin;
use crate::config_store::{self, RulesView};
use crate::logcat::{self, LogcatParser};
use crate::model::{
    DeviceCaptureOptions, FileInfo, FilterSpec, LogEntry, QueryResult, Rule, RuleCategory,
    StreamStatus,
};
use crate::rules::CompiledFilter;
use crate::session::{AppState, LogSession};
use crate::stream::DeviceStream;
use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};

const MAX_OPEN_SESSIONS: usize = 4;

/// Unique-ish suffix for freshly generated rule / category ids.
fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Everything that identifies a session's origin, separate from the parsed content.
struct SessionSeed {
    path: String,
    file_name: String,
    size_bytes: u64,
    /// `file` | `stream`
    source: &'static str,
    device: String,
    streaming: bool,
}

fn register_session(
    state: &State<'_, AppState>,
    entries: Vec<LogEntry>,
    total_lines: u32,
    unparsed_lines: u32,
    seed: SessionSeed,
) -> Result<FileInfo, String> {
    let (levels, tags, pids, packages) = logcat::collect_facets(&entries);
    let info = FileInfo {
        session_id: state.next_id(),
        path: seed.path,
        file_name: seed.file_name,
        size_bytes: seed.size_bytes,
        total_lines,
        entry_count: entries.len() as u32,
        unparsed_lines,
        levels,
        tags,
        pids,
        packages,
        source: seed.source.to_string(),
        device: seed.device,
        streaming: seed.streaming,
    };

    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    if sessions.len() >= MAX_OPEN_SESSIONS {
        // `HashMap` iteration order is arbitrary, so "the oldest" has to be derived from the
        // id itself — ids come from a monotonic counter (`s0`, `s1`, ...). The UI caps tabs
        // at the same number, so this is a safety net for leaked sessions (e.g. a webview
        // reload that orphaned them), not a routine path.
        let oldest = sessions
            .keys()
            .min_by_key(|id| id.trim_start_matches('s').parse::<u64>().unwrap_or(u64::MAX))
            .cloned();
        if let Some(oldest) = oldest {
            sessions.remove(&oldest);
            // An evicted session must not leave its `adb logcat` running.
            if let Ok(mut streams) = state.streams.lock() {
                if let Some(mut stream) = streams.remove(&oldest) {
                    stream.stop();
                }
            }
        }
    }
    sessions.insert(info.session_id.clone(), LogSession::new(entries));
    Ok(info)
}

#[tauri::command]
pub fn open_log_file(path: String, state: State<'_, AppState>) -> Result<FileInfo, String> {
    let file_path = Path::new(&path);
    let meta = fs::metadata(file_path).map_err(|e| format!("无法读取文件信息: {e}"))?;
    if !meta.is_file() {
        return Err("所选路径不是文件".to_string());
    }
    let parsed = LogcatParser::parse_file(file_path)?;
    register_session(
        &state,
        parsed.entries,
        parsed.total_lines,
        parsed.unparsed_lines,
        SessionSeed {
            path: path.clone(),
            file_name: file_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
            size_bytes: meta.len(),
            source: "file",
            device: String::new(),
            streaming: false,
        },
    )
}

/// `adb version` + resolved path. Never fails: a missing adb is reported in the payload so
/// the UI can show install guidance instead of a red error banner.
#[tauri::command]
pub fn adb_status() -> adb::AdbStatus {
    adb::status()
}

#[tauri::command]
pub fn list_adb_devices() -> Result<Vec<adb::AdbDevice>, String> {
    adb::devices()
}

fn require_serial(options: &DeviceCaptureOptions) -> Result<String, String> {
    let serial = options.serial.trim().to_string();
    if serial.is_empty() {
        return Err("请先选择一台已授权的设备".to_string());
    }
    Ok(serial)
}

/// Best-effort model name for the captured session, so the top bar shows something more
/// useful than a serial number.
fn device_label(serial: &str) -> String {
    adb::devices()
        .ok()
        .and_then(|devices| {
            devices
                .into_iter()
                .find(|d| d.serial == serial)
                .map(|d| d.label())
        })
        .unwrap_or_else(|| serial.to_string())
}

/// Refuse to start a capture against a device that is visibly not ready (unauthorized,
/// offline, missing udev rules). If the device list itself cannot be read we stay out of
/// the way and let `adb logcat` report the real problem.
fn require_ready(serial: &str) -> Result<(), String> {
    let Ok(devices) = adb::devices() else {
        return Ok(());
    };
    match devices.into_iter().find(|d| d.serial == serial) {
        Some(device) if device.is_ready() => Ok(()),
        Some(device) => Err(format!(
            "设备 {} 当前状态是「{}」。请先在手机上确认 USB 调试授权，或重新插拔数据线。",
            device.serial, device.state
        )),
        None => Err(format!(
            "设备 {serial} 已不在连接列表中，请刷新设备列表后重试"
        )),
    }
}

/// Refuse a second live capture of the same device.
///
/// Two `adb logcat` children would fight over one ring buffer (each replays and then
/// follows the same lines, doubling the load), and `clear_before` on the second one would
/// wipe the lines the first capture is in the middle of collecting. The UI checks this too
/// so it can point at the existing tab, but the backend check is the authoritative one.
fn ensure_device_not_streaming(state: &State<'_, AppState>, serial: &str) -> Result<(), String> {
    let streams = state
        .streams
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    if streams.values().any(|stream| stream.serial() == serial) {
        return Err(format!(
            "设备 {serial} 已经在一个会话里实时抓取了。同一台设备同时只能有一个实时抓取，请先在那个标签点「停止实时抓取」再重新开始。"
        ));
    }
    Ok(())
}

/// Start following new device log lines until [`stop_device_stream`] is called.
#[tauri::command]
pub fn start_device_stream(
    options: DeviceCaptureOptions,
    state: State<'_, AppState>,
) -> Result<FileInfo, String> {
    let serial = require_serial(&options)?;
    let adb_path = adb::locate().ok_or_else(|| adb::NOT_FOUND.to_string())?;
    require_ready(&serial)?;
    // Must run before `clear_before`: clearing the buffer of a device that another session
    // is capturing would destroy that session's data.
    ensure_device_not_streaming(&state, &serial)?;
    if options.clear_before {
        adb::clear_logcat(&adb_path, &serial, &options.buffers)?;
    }

    let label = device_label(&serial);
    let info = register_session(
        &state,
        Vec::new(),
        0,
        0,
        SessionSeed {
            path: format!("adb://{serial}/logcat"),
            file_name: format!("{label} · 实时 logcat"),
            size_bytes: 0,
            source: "stream",
            device: serial.clone(),
            streaming: true,
        },
    )?;

    let args = adb::logcat_args(&serial, &options.buffers);
    let mut stream = match DeviceStream::spawn(&adb_path, args, &serial, info.session_id.clone()) {
        Ok(stream) => stream,
        Err(e) => {
            remove_session(&state, &info.session_id);
            return Err(e);
        }
    };
    // `adb` fails fast for an unauthorized / offline device. Catching that here gives a
    // real error instead of a session that silently stays empty forever.
    if let Some(error) = stream.startup_error() {
        stream.stop();
        remove_session(&state, &info.session_id);
        return Err(error);
    }

    // Re-check under the lock that actually publishes the stream: the earlier check ran
    // before `spawn`, which takes long enough for a concurrent call to slip through.
    {
        let mut streams = state
            .streams
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        if streams.values().any(|existing| existing.serial() == serial) {
            drop(streams);
            stream.stop();
            remove_session(&state, &info.session_id);
            return Err(format!(
                "设备 {serial} 已经在一个会话里实时抓取了。同一台设备同时只能有一个实时抓取。"
            ));
        }
        streams.insert(info.session_id.clone(), stream);
    }
    Ok(info)
}

#[tauri::command]
pub fn poll_device_stream(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<StreamStatus, String> {
    // Two short-lived locks, never nested: the reader thread only ever touches its own
    // buffer, so holding both while parsing would buy nothing and risk a lock cycle.
    let (pending, mut status) = {
        let mut streams = state
            .streams
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        let stream = streams
            .get_mut(&session_id)
            .ok_or_else(|| "实时抓取已停止".to_string())?;
        stream.drain()
    };

    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "日志会话已失效，请重新抓取".to_string())?;
    let before = session.entries.len();
    status.entry_count = session.append_all(pending);
    // New tags / packages must show up in the filter dropdowns while the capture runs.
    // Recomputing only on growth keeps idle polls cheap.
    if session.entries.len() > before {
        let (levels, tags, pids, packages) = logcat::collect_facets(&session.entries);
        status.facets_updated = true;
        status.levels = levels;
        status.tags = tags;
        status.pids = pids;
        status.packages = packages;
    }
    Ok(status)
}

#[tauri::command]
pub fn stop_device_stream(session_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut streams = state
        .streams
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    if let Some(mut stream) = streams.remove(&session_id) {
        stream.stop();
    }
    Ok(())
}

fn remove_session(state: &State<'_, AppState>, session_id: &str) {
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.remove(session_id);
    }
}


#[tauri::command]
pub fn query_logs(
    session_id: String,
    filter: FilterSpec,
    offset: u32,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<QueryResult, String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "日志会话已失效，请重新打开文件".to_string())?;
    session.query(&filter, offset, limit)
}

#[tauri::command]
pub fn close_session(session_id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Ok(mut streams) = state.streams.lock() {
        if let Some(mut stream) = streams.remove(&session_id) {
            stream.stop();
        }
    }
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    sessions.remove(&session_id);
    Ok(())
}

#[tauri::command]
pub fn export_filtered(
    session_id: String,
    filter: FilterSpec,
    dest_path: String,
    state: State<'_, AppState>,
) -> Result<u32, String> {
    let lines: Vec<String> = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| "日志会话已失效，请重新打开文件".to_string())?;
        let compiled = CompiledFilter::compile(&filter)?;
        let mut indices = session.matched_indices(&filter, &compiled);
        // Export respects the on-screen order (the UI can sort descending).
        if filter.order.eq_ignore_ascii_case("desc") {
            indices.reverse();
        }
        indices
            .iter()
            .filter_map(|&i| session.entries.get(i as usize).map(|e| e.raw.clone()))
            .collect()
    };

    let file = fs::File::create(&dest_path).map_err(|e| format!("无法创建导出文件: {e}"))?;
    let mut writer = BufWriter::new(file);
    for line in &lines {
        writeln!(writer, "{line}").map_err(|e| format!("写入导出文件失败: {e}"))?;
    }
    writer.flush().map_err(|e| format!("写入导出文件失败: {e}"))?;
    Ok(lines.len() as u32)
}

#[tauri::command]
pub fn list_builtin_templates() -> Vec<Rule> {
    builtin::templates()
}

/// All user-defined rules plus the full category list (built-in + user) in one round trip.
#[tauri::command]
pub fn list_rule_bundle(app: AppHandle) -> Result<RulesView, String> {
    Ok(config_store::view(&config_store::load(&app)?))
}

#[tauri::command]
pub fn rules_location(app: AppHandle) -> Result<String, String> {
    config_store::location(&app)
}

#[tauri::command]
pub fn save_custom_rule(app: AppHandle, rule: Rule) -> Result<RulesView, String> {
    if rule.id.starts_with("builtin-") || rule.builtin {
        return Err("内置模板不可修改，请另存为自定义规则".to_string());
    }
    if rule.name.trim().is_empty() {
        return Err("规则名称不能为空".to_string());
    }
    let rules = config_store::load(&app)?.rules;
    config_store::save_rules(&app, &upsert_by_id(rules, rule))
}

fn upsert_by_id(mut rules: Vec<Rule>, rule: Rule) -> Vec<Rule> {
    if let Some(existing) = rules.iter_mut().find(|r| r.id == rule.id) {
        *existing = rule;
    } else {
        rules.push(rule);
    }
    rules
}

#[tauri::command]
pub fn delete_custom_rule(app: AppHandle, id: String) -> Result<RulesView, String> {
    let mut file = config_store::load(&app)?;
    file.rules.retain(|r| r.id != id);
    Ok(config_store::view(&config_store::save(&app, &file)?))
}

/// Built-in categories are read-only; user categories are free-form classifications.
/// The editor sends the whole category, so an empty / colliding id is regenerated here.
#[tauri::command]
pub fn save_rule_category(app: AppHandle, category: RuleCategory) -> Result<RulesView, String> {
    if category.builtin || config_store::builtin_category_ids().contains(&category.id) {
        return Err("内置分组不可修改，请新建一个分组".to_string());
    }
    if category.name.trim().is_empty() {
        return Err("分组名称不能为空".to_string());
    }
    let millis = now_millis();
    let mut category = category;
    category.builtin = false;
    if category.id.trim().is_empty() {
        category.id = format!("custom-cat-{millis}");
    }
    let mut file = config_store::load(&app)?;
    let mut categories = std::mem::take(&mut file.categories);
    if let Some(existing) = categories.iter_mut().find(|c| c.id == category.id) {
        *existing = category;
    } else {
        categories.push(category);
    }
    config_store::save_categories(&app, &categories)
}

/// Deleting a category never deletes its rules — they fall back to "uncategorised".
#[tauri::command]
pub fn delete_rule_category(app: AppHandle, id: String) -> Result<RulesView, String> {
    if config_store::builtin_category_ids().contains(&id) {
        return Err("内置分组不可删除".to_string());
    }
    let mut file = config_store::load(&app)?;
    file.categories.retain(|c| c.id != id);
    for rule in &mut file.rules {
        if rule.category_id == id {
            rule.category_id.clear();
        }
    }
    Ok(config_store::view(&config_store::save(&app, &file)?))
}

fn export_document(app: &AppHandle) -> Result<String, String> {
    let mut file = config_store::load(app)?;
    file.version = config_store::RULES_FILE_VERSION;
    serde_json::to_string_pretty(&file).map_err(|e| format!("导出规则失败: {e}"))
}

#[tauri::command]
pub fn export_rules_json(app: AppHandle) -> Result<String, String> {
    export_document(&app)
}

/// Write rules + their categories to a user-chosen file (dialog is handled on the JS side).
#[tauri::command]
pub fn export_rules_to_file(app: AppHandle, dest_path: String) -> Result<u32, String> {
    let file = config_store::load(&app)?;
    let text = export_document(&app)?;
    fs::write(&dest_path, text).map_err(|e| format!("写入规则文件失败: {e}"))?;
    Ok(file.rules.len() as u32)
}

#[tauri::command]
pub fn import_rules_from_file(app: AppHandle, path: String) -> Result<RulesView, String> {
    let text = fs::read_to_string(&path).map_err(|e| format!("读取规则文件失败: {e}"))?;
    import_rules_json(app, text)
}

#[tauri::command]
pub fn import_rules_json(app: AppHandle, json: String) -> Result<RulesView, String> {
    // Accepts the v2 document, the v1 document, and the bare rule array from older exports.
    let incoming = config_store::parse_document(&json)?;
    let millis = now_millis();
    let builtin_ids = config_store::builtin_category_ids();
    let mut file = config_store::load(&app)?;

    let mut cat_seen: HashMap<String, usize> = HashMap::new();
    // Imported category ids may be regenerated (empty / builtin clash / duplicate).
    // Remember the mapping so imported rules still point at their category.
    let mut cat_id_map: HashMap<String, String> = HashMap::new();
    for (i, mut category) in incoming.categories.into_iter().enumerate() {
        if category.name.trim().is_empty() {
            continue;
        }
        let original_id = category.id.clone();
        category.builtin = false;
        if category.id.trim().is_empty()
            || builtin_ids.contains(&category.id)
            || category.id.starts_with("builtin-")
        {
            category.id = format!("custom-cat-{millis}-{i}");
        }
        let occurrence = cat_seen.entry(category.id.clone()).or_insert(0);
        if *occurrence > 0 {
            category.id = format!("{}-{millis}-{i}", category.id);
        }
        *occurrence += 1;
        cat_id_map.insert(original_id, category.id.clone());
        if let Some(existing) = file.categories.iter_mut().find(|c| c.id == category.id) {
            *existing = category;
        } else {
            file.categories.push(category);
        }
    }

    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, mut rule) in incoming.rules.into_iter().enumerate() {
        if rule.name.trim().is_empty() || rule.group.conditions.is_empty() {
            continue;
        }
        rule.builtin = false;
        if let Some(new_id) = cat_id_map.get(&rule.category_id) {
            rule.category_id = new_id.clone();
        }
        if rule.id.trim().is_empty() || rule.id.starts_with("builtin-") {
            rule.id = format!("custom-{millis}-{i}");
        }
        // Imported rules replace same-id rules instead of creating duplicates.
        let occurrence = seen.entry(rule.id.clone()).or_insert(0);
        if *occurrence > 0 {
            rule.id = format!("{}-{millis}-{i}", rule.id);
        }
        *occurrence += 1;
        if let Some(existing) = file.rules.iter_mut().find(|r| r.id == rule.id) {
            *existing = rule;
        } else {
            file.rules.push(rule);
        }
    }
    // `save` normalises dangling categoryId values against the merged category list.
    Ok(config_store::view(&config_store::save(&app, &file)?))
}
