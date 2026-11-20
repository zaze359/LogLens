//! Tauri IPC commands. All file IO happens here in Rust, so no filesystem plugin scope
//! has to be configured for arbitrary user-selected log files.

use crate::adb;
use crate::builtin;
use crate::config_store::{self, RulesView};
use crate::logcat::{self, LogcatParser, ParseOutcome};
use crate::model::{
    CdpCaptureOptions, DeviceCaptureOptions, FileInfo, FilterSpec, LogEntry, NetworkPollResult,
    NetworkSessionInfo, OpenProgress, ProxyCaptureOptions, ProxyState, QueryResult, Rule,
    RuleCategory, SearchHistoryEntry, SearchHits, StreamStatus,
};
use crate::rules::CompiledFilter;
use crate::session::{AppState, LogSession, NetworkSession};
use crate::stream::DeviceStream;
use crate::{
    netcapture::{self, AnyCapture, NetworkCapture, ProxyGuard},
    netprobe, network,
    proxy::{proxy_status, ProxyCapture},
};
use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

const MAX_OPEN_SESSIONS: usize = 4;

/// How long a live stream may go without a poll before it is treated as orphaned.
///
/// The webview polls every 800ms. Five seconds tolerates a stalled IPC or a suspended
/// window, but still frees a device within a few seconds of a closed tab / webview reload.
const STREAM_LEASE: Duration = Duration::from_secs(5);

/// Event name used for import progress. The payload is [`OpenProgress`].
const IMPORT_PROGRESS_EVENT: &str = "log-open-progress";

/// How often progress is pushed to the webview. Ten events per second is already smoother
/// than the eye can follow; going higher would spend more time serialising JSON than parsing.
const IMPORT_PROGRESS_MS: u64 = 100;

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

/// Distinct levels / tags / pids / packages, in that order.
type Facets = (Vec<String>, Vec<String>, Vec<String>, Vec<String>);

fn register_session(
    state: &State<'_, AppState>,
    entries: Vec<LogEntry>,
    total_lines: u32,
    unparsed_lines: u32,
    seed: SessionSeed,
) -> Result<FileInfo, String> {
    let facets = logcat::collect_facets(&entries);
    register_session_with_facets(state, entries, total_lines, unparsed_lines, facets, seed)
}

/// Registration with pre-computed facets, so a file import can build them on the parsing
/// thread (see `open_log_file`) instead of stalling right after the progress bar fills up.
fn register_session_with_facets(
    state: &State<'_, AppState>,
    entries: Vec<LogEntry>,
    total_lines: u32,
    unparsed_lines: u32,
    facets: Facets,
    seed: SessionSeed,
) -> Result<FileInfo, String> {
    let (levels, tags, pids, packages) = facets;
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

/// Imports a log file.
///
/// Async on purpose. A synchronous command parses on the IPC thread, so a 200 MB log freezes
/// the whole window until it finishes — which is indistinguishable from a hang. Here the read
/// runs on the blocking pool, reports progress roughly every `IMPORT_PROGRESS_MS`, and the
/// webview keeps painting (the user can cancel, switch tabs, or start a live capture).
///
/// `request_id` is generated by the caller and echoed in every progress event, so several
/// imports in flight never apply each other's numbers.
#[tauri::command]
pub async fn open_log_file(
    path: String,
    request_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FileInfo, String> {
    let file_path = Path::new(&path).to_path_buf();
    let meta = fs::metadata(&file_path).map_err(|e| format!("无法读取文件信息: {e}"))?;
    if !meta.is_file() {
        return Err("所选路径不是文件".to_string());
    }
    let total_bytes = meta.len();
    let file_name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    capture_flow(
        "file.open",
        "开始读取日志",
        &format!("requestId={request_id} file={file_name} bytes={total_bytes}"),
    );

    // Registering the flag before the read starts is what makes `cancel_open_log` able to
    // reach a parse that is already underway.
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut cancels = state
            .import_cancels
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        if let Some(previous) = cancels.insert(request_id.clone(), cancel.clone()) {
            // Same id reused (UI restarted the import): stop the older parse instead of
            // letting two readers race for one request.
            previous.store(true, Ordering::Relaxed);
        }
    }

    let app_handle = app.clone();
    let rid = request_id.clone();
    let parse_path = file_path.clone();
    // The worker gets its own handle so the caller keeps one to check against at the end.
    let worker_cancel = cancel.clone();
    let outcome = tauri::async_runtime::spawn_blocking(
        move || -> Result<(ParseOutcome, Option<Facets>), String> {
            let progress = |bytes: u64, entries: u32, phase: &'static str| {
                let _ = app_handle.emit(
                    IMPORT_PROGRESS_EVENT,
                    OpenProgress {
                        request_id: rid.clone(),
                        bytes_read: bytes,
                        total_bytes,
                        entries,
                        phase: phase.to_string(),
                    },
                );
            };
            // Emit before the first line so the card can show the file size immediately
            // instead of an empty bar while a slow disk warms up.
            progress(0, 0, "parsing");
            let mut last_emit = Instant::now();
            let parsed = LogcatParser::parse_file_with_progress(&parse_path, |bytes, entries| {
                if worker_cancel.load(Ordering::Relaxed) {
                    return false;
                }
                if last_emit.elapsed() >= Duration::from_millis(IMPORT_PROGRESS_MS) {
                    last_emit = Instant::now();
                    progress(bytes, entries, "parsing");
                }
                true
            })?;
            match parsed {
                ParseOutcome::Cancelled => Ok((ParseOutcome::Cancelled, None)),
                ParseOutcome::Done(parsed) => {
                    // Facet lists are another full pass over every entry. Building them here
                    // keeps the bar from freezing at 100% with no explanation.
                    progress(total_bytes, parsed.entries.len() as u32, "indexing");
                    let facets = logcat::collect_facets(&parsed.entries);
                    Ok((ParseOutcome::Done(parsed), Some(facets)))
                }
            }
        },
    )
    .await
    .map_err(|e| format!("导入线程异常: {e}"))?;

    if let Ok(mut cancels) = state.import_cancels.lock() {
        cancels.remove(&request_id);
    }

    let (outcome, facets) = outcome?;
    // A cancel that arrives after the last chunk has already been read still has to win:
    // otherwise clicking 取消 at 99% would open a tab anyway.
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消导入".to_string());
    }
    match outcome {
        ParseOutcome::Cancelled => Err("已取消导入".to_string()),
        ParseOutcome::Done(parsed) => {
            let lines = parsed.total_lines;
            let entries = parsed.entries.len() as u32;
            let unparsed = parsed.unparsed_lines;
            let info = register_session_with_facets(
                &state,
                parsed.entries,
                lines,
                unparsed,
                facets.unwrap_or_default(),
                SessionSeed {
                    path,
                    file_name,
                    size_bytes: total_bytes,
                    source: "file",
                    device: String::new(),
                    streaming: false,
                },
            )?;
            capture_flow(
                "file.open",
                "日志导入完成",
                &format!("lines={lines} entries={entries} unparsed={unparsed}"),
            );
            Ok(info)
        }
    }
}

/// Aborts an in-flight import. The parse thread notices the flag on its next chunk and
/// returns without registering a session, so there is nothing to roll back.
#[tauri::command]
pub fn cancel_open_log(request_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let cancels = state
        .import_cancels
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    if let Some(flag) = cancels.get(&request_id) {
        flag.store(true, Ordering::Relaxed);
        capture_flow("file.open", "取消日志导入", &format!("requestId={request_id}"));
    }
    Ok(())
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

/// Stop streams for `serial` whose webview has stopped polling them, and drop their
/// sessions. Scoped to one device on purpose: a global sweep could kill a *different*
/// device's live capture that merely hasn't renewed its lease yet (e.g. the machine just
/// woke from sleep). Reaching this function means the frontend has no busy tab for this
/// serial, so reclaiming this serial's stale streams is always safe.
///
/// Locks are never nested: take the stream ids out under the stream lock, release it, then
/// drop the sessions. Holding both would invert the order used by `register_session`.
fn reap_orphan_streams(state: &State<'_, AppState>, serial: &str) -> Result<(), String> {
    let orphan_ids: Vec<String> = {
        let mut streams = state
            .streams
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        let mut ids = Vec::new();
        streams.retain(|id, stream| {
            if stream.serial() == serial && stream.idle_for() >= STREAM_LEASE {
                stream.stop();
                ids.push(id.clone());
                false
            } else {
                true
            }
        });
        ids
    };
    if !orphan_ids.is_empty() {
        capture_flow(
            "device.stream",
            "自愈已租约过期的实时流",
            &format!("serial={serial} streams={}", orphan_ids.len()),
        );
        if let Ok(mut sessions) = state.sessions.lock() {
            for id in &orphan_ids {
                sessions.remove(id);
            }
        }
    }
    Ok(())
}

/// Refuse a second live capture of a device that still has a tab polling it.
///
/// Two `adb logcat` children would fight over one ring buffer (each replays and then
/// follows the same lines, doubling the load), and `clear_before` on the second one would
/// wipe the lines the first capture is in the middle of collecting. Orphans are reclaimed
/// first, so a stale stream never blocks a device that has no live tab.
fn ensure_device_not_streaming(state: &State<'_, AppState>, serial: &str) -> Result<(), String> {
    reap_orphan_streams(state, serial)?;
    let streams = state
        .streams
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    if streams.values().any(|stream| stream.serial() == serial) {
        return Err(format!(
            "设备 {serial} 的另一份日志正在实时抓取。请切到那份日志点「停止抓取」后再重新开始。"
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
                "设备 {serial} 的另一份日志正在实时抓取。请切到那份日志点「停止抓取」后再重新开始。"
            ));
        }
        streams.insert(info.session_id.clone(), stream);
    }
    capture_flow(
        "device.stream",
        "后端实时抓取已启动",
        &format!("id={} serial={}", info.session_id, serial),
    );
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
        capture_flow(
            "device.stream",
            "后端停止实时抓取",
            &format!("id={session_id} serial={}", stream.serial()),
        );
    }
    Ok(())
}

/// Force-release every live stream attached to a serial, regardless of lease age.
///
/// Only called by the frontend when it has already confirmed it holds no tab for that
/// device — e.g. right after a webview reload, before the 5s lease has lapsed. Without
/// this the user would be told to switch to a tab that no longer exists. Each stream's
/// `adb logcat` child is killed and its session dropped so the serial is free at once.
#[tauri::command]
pub fn release_device_stream(serial: String, state: State<'_, AppState>) -> Result<(), String> {
    let orphan_ids: Vec<String> = {
        let mut streams = state
            .streams
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        let mut ids = Vec::new();
        streams.retain(|id, stream| {
            if stream.serial() == serial.trim() {
                stream.stop();
                ids.push(id.clone());
                false
            } else {
                true
            }
        });
        ids
    };
    if !orphan_ids.is_empty() {
        capture_flow(
            "device.stream",
            "强制释放残留实时流",
            &format!("serial={serial} streams={}", orphan_ids.len()),
        );
        if let Ok(mut sessions) = state.sessions.lock() {
            for id in &orphan_ids {
                sessions.remove(id);
            }
        }
    }
    Ok(())
}

fn remove_session(state: &State<'_, AppState>, session_id: &str) {
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.remove(session_id);
    }
}


#[tauri::command]
pub fn search_hits(
    session_id: String,
    filter: FilterSpec,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<SearchHits, String> {
    // "Find" is deliberately a separate command from `query_logs`: it must not be able to
    // change which rows are listed, only report where the term sits among them.
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "日志会话已失效，请重新打开文件".to_string())?;
    session.find(&filter, limit)
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
    capture_flow("session.close", "关闭日志会话", &format!("id={session_id}"));
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
    let count = lines.len() as u32;
    capture_flow(
        "export.log",
        "筛选日志已导出",
        &format!("id={session_id} rows={count}"),
    );
    Ok(count)
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

// --- Device network capture -------------------------------------------------

/// Preferred local port for a CDP capture. Different from `netprobe::PROBE_PORT` so a
/// one-shot capability probe never steals the port of a running capture.
///
/// Both ports below are **preferences only**: the local end of a capture is a single
/// machine-wide resource while captures are per device, so when the number is already
/// taken (a capture left on an unplugged phone, a second device, another app) the capture
/// moves to an OS-assigned port instead of failing with `Address already in use`.
const CDP_PORT: u16 = 19333;
/// Preferred local port for the built-in proxy. Distinct from CDP_PORT and
/// netprobe::PROBE_PORT so the three channels can run at the same time without stealing
/// each other's local port.
const PROXY_PORT: u16 = 18999;

fn require_adb() -> Result<std::path::PathBuf, String> {
    adb::locate().ok_or_else(|| adb::NOT_FOUND.to_string())
}

fn network_info(id: &str, session: &NetworkSession) -> NetworkSessionInfo {
    NetworkSessionInfo {
        id: id.to_string(),
        label: session.label.clone(),
        source: session.source.clone(),
        device: session.device.clone(),
        streaming: session.streaming,
        count: session.requests.len() as u32,
        device_ip: session.device_ip.clone(),
    }
}

/// Third-party packages installed on the device, for the "which app do you want to
/// inspect" picker in the network panel.
#[tauri::command]
pub fn list_capture_packages(serial: String) -> Result<Vec<String>, String> {
    let adb_path = require_adb()?;
    Ok(netprobe::list_packages(&adb_path, serial.trim()))
}

/// Read-only capability probe: which routes to HTTPS plaintext are open on this device.
#[tauri::command]
pub fn probe_network_capture(
    serial: String,
    packages: Vec<String>,
) -> Result<netprobe::ProbeReport, String> {
    let adb_path = require_adb()?;
    // Re-running a probe also heals a proxy left by a previous run, so reopening
    // the panel recovers the device without needing an app restart.
    let healed = netcapture::heal_stale_proxies();
    let mut report = netprobe::probe(&adb_path, serial.trim(), &packages)?;
    report.healed_proxy = healed;
    Ok(report)
}

/// One bounded checkpoint in the capture pipeline. The UI can show `streaming: true`
/// while a row is stuck in any of: reader → buffer → poll → merge → render. Counters at
/// each boundary make the actual boundary obvious; never put a URL or body in here.
fn capture_flow(scope: &str, message: &str, detail: &str) {
    crate::error_log::record_flow(scope, message, detail);
}

/// Remove one channel's captures that an earlier webview left behind on this device.
///
/// A webview reload (hot update / manual reload) or a frontend crash drops the ids the UI
/// used to poll, but the backend still owns the device proxy / `adb forward`. Without this
/// the next "start" would be refused forever by the "already running" check, with no way to
/// stop the invisible capture. The panel keeps at most one capture per device and channel,
/// so replacing the orphan is always the right recovery.
fn take_over_stale_captures(state: &AppState, serial: &str, proxy: bool) -> Vec<String> {
    let Ok(mut captures) = state.network_captures.lock() else {
        return Vec::new();
    };
    let stale: Vec<String> = captures
        .iter()
        .filter(|(_, capture)| capture.is_proxy() == proxy && capture.serial() == serial)
        .map(|(id, _)| id.clone())
        .collect();
    let mut stopped = Vec::new();
    for id in stale {
        if let Some(mut capture) = captures.remove(&id) {
            capture_flow(
                &format!("capture.stop.{id}"),
                if proxy {
                    "替换上一次残留的原生采集"
                } else {
                    "替换上一次残留的 H5 采集"
                },
                &format!("id={id} serial={serial}"),
            );
            if let Err(e) = capture.stop() {
                capture_flow(
                    &format!("capture.stop.{id}"),
                    "清理残留采集失败，已交给后台重试",
                    &e,
                );
            }
            stopped.push(id);
        }
    }
    stopped
}

#[tauri::command]
pub fn start_cdp_capture(
    options: CdpCaptureOptions,
    state: State<'_, AppState>,
) -> Result<NetworkSessionInfo, String> {
    let serial = options.serial.trim().to_string();
    if serial.is_empty() {
        return Err("请先选择一台已授权的设备".to_string());
    }
    let adb_path = require_adb()?;
    let socket = if options.socket.trim().is_empty() {
        netprobe::probe(&adb_path, &serial, &[])
            .ok()
            .and_then(|r| r.evidence.devtools.first().map(|d| d.socket.clone()))
            .ok_or_else(|| "设备上没有 WebView 调试通道，请先运行环境体检".to_string())?
    } else {
        options.socket.trim().to_string()
    };
    let port = if options.port == 0 { CDP_PORT } else { options.port };
    capture_flow(
        "capture.start",
        "请求启动 H5 采集",
        &format!("serial={serial} socket={socket} port={port}"),
    );

    // One CDP capture per device. The panel follows the device's first devtools socket, so
    // a second capture would duplicate every row and fight over the same `adb forward`.
    // Comparing by serial rather than by port also keeps this correct now that `start_cdp`
    // may move to a fallback port when the preferred one is taken.
    let replaced = take_over_stale_captures(&state, &serial, false);
    if !replaced.is_empty() {
        if let Ok(mut sessions) = state.network_sessions.lock() {
            for id in &replaced {
                sessions.remove(id);
            }
        }
    }

    let id = format!("net-{}", state.next_id());
    let capture = NetworkCapture::start_cdp(
        &adb_path,
        &serial,
        &socket,
        port,
        netcapture::device_tz_offset(&adb_path, &serial),
        id.clone(),
    )?;
    let session = NetworkSession {
        requests: Vec::new(),
        device_ip: String::new(),
        label: format!("{} · WebView 请求", device_label(&serial)),
        source: "cdp".to_string(),
        device: serial.clone(),
        streaming: true,
    };
    let info = network_info(&id, &session);
    capture_flow(
        "capture.start",
        "H5 采集已就绪",
        &format!("id={id} serial={serial} socket={socket} port={port}"),
    );
    state
        .network_sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?
        .insert(id.clone(), session);
    state
        .network_captures
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?
        .insert(id, AnyCapture::Cdp(capture));
    Ok(info)
}

/// Start the built-in proxy for native (OkHttp / Cronet / system) traffic.
///
/// The device proxy is set only after the listener is ready, and the returned capture
/// owns the [`ProxyGuard`]: stopping it (or closing the window) clears the device proxy,
/// so a crash cannot leave the phone dialling a dead port.
#[tauri::command]
pub fn start_proxy_capture(
    options: ProxyCaptureOptions,
    state: State<'_, AppState>,
) -> Result<NetworkSessionInfo, String> {
    let serial = options.serial.trim().to_string();
    if serial.is_empty() {
        return Err("请先选择一台已授权的设备".to_string());
    }
    let adb_path = require_adb()?;
    // The listen port belongs to the proxy feature; a device capture only binds to whatever
    // endpoint that proxy already publishes. Fail early with a message that names the fix,
    // so the UI never has to guess why a "start" did nothing.
    {
        let runtime = state
            .proxy_runtime
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        if !runtime.running() {
            return Err(
                "还没有开启代理。请先在顶栏点「代理」并开启，再回来开始原生接口采集。".to_string(),
            );
        }
    }
    capture_flow("capture.start", "请求启动原生采集", &format!("serial={serial}"));

    // The checks below are check-then-act, and `ProxyCapture::start` does several `adb`
    // round trips afterwards. Two rapid clicks (or two windows) used to be caught by the
    // port conflict; now that a taken port falls back instead of failing, only this
    // reservation keeps a device from ending up with two guards and no way to restore.
    let Some(_starting) = state.begin_proxy_start(&serial) else {
        return Err("该设备正在启动原生接口采集，请稍候再试".to_string());
    };

    let replaced: Vec<String> = {
        let guards = state
            .proxy_guards
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        if guards.contains_key(&serial) {
            return Err("该设备已有手动设置的代理。请先在「MITM 代理设置」里点「还原」，再开始原生接口采集。".to_string());
        }
        // 前端刷新 / 崩溃后，这台设备上一轮的原生采集还在后端跑，而前端已经丢了它的 id，
        // 既停不掉也关不掉。再点开始时直接替换，否则会永远卡在「已经在运行」。
        take_over_stale_captures(&state, &serial, true)
    };
    if !replaced.is_empty() {
        if let Ok(mut sessions) = state.network_sessions.lock() {
            for id in &replaced {
                sessions.remove(id);
            }
        }
    }

    let id = format!("net-{}", state.next_id());
    let capture = ProxyCapture::start(
        &state.proxy_runtime,
        &adb_path,
        &serial,
        netcapture::device_tz_offset(&adb_path, &serial),
        id.clone(),
    )?;
    let endpoint = capture.info().endpoint.clone();
    let device_ip = capture.info().device_ip;
    let session = NetworkSession {
        requests: Vec::new(),
        device_ip,
        label: format!("{} · 原生接口", device_label(&serial)),
        source: "proxy".to_string(),
        device: serial.clone(),
        streaming: true,
    };
    let info = NetworkSessionInfo {
        label: format!("{}（{}）", session.label, endpoint),
        ..network_info(&id, &session)
    };
    capture_flow(
        "capture.start",
        "原生采集已就绪",
        &format!("id={id} serial={serial} endpoint={endpoint}"),
    );
    state
        .network_sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?
        .insert(id.clone(), session);
    state
        .network_captures
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?
        .insert(id, AnyCapture::Proxy(capture));
    Ok(info)
}

/// Drain the CDP reader into the traffic session and return the full current list.
#[tauri::command]
pub fn poll_network_capture(
    id: String,
    state: State<'_, AppState>,
) -> Result<NetworkPollResult, String> {
    let (pending, mut status) = {
        let mut captures = state
            .network_captures
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        let capture = captures
            .get_mut(&id)
            .ok_or_else(|| "实时采集已停止".to_string())?;
        capture.drain()
    };
    let added = pending.len() as u32;
    let mut sessions = state
        .network_sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| "网络请求会话已失效".to_string())?;
    let source = session.source.clone();
    let serial = session.device.clone();
    let first_batch = session.requests.is_empty() && added > 0;
    for mut request in pending {
        request.index = session.requests.len() as u32;
        session.requests.push(request);
    }
    status.entry_count = session.requests.len() as u32;
    if first_batch {
        // A clear success marker: with only "poll" rows in the log, "did any traffic
        // ever arrive?" is impossible to answer at a glance.
        capture_flow(
            "capture.rows",
            "收到第一批请求",
            &format!(
                "id={id} source={source} serial={serial} rows={} events={}",
                status.entry_count, status.events
            ),
        );
    }
    capture_flow(
        &format!("capture.poll.{id}"),
        "后端快照",
        &format!(
            "id={id} source={source} serial={serial} added={added} rows={} events={} running={} error={}",
            status.entry_count, status.events, status.running, status.error
        ),
    );
    if !status.running {
        session.streaming = false;
    }
    Ok(NetworkPollResult {
        id,
        requests: session.requests.clone(),
        status,
        added,
    })
}

#[tauri::command]
pub fn query_network_requests(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::network::NetworkRequest>, String> {
    let sessions = state
        .network_sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    Ok(sessions
        .get(&id)
        .map(|s| s.requests.clone())
        .unwrap_or_default())
}

#[tauri::command]
pub fn clear_network_capture(id: String, state: State<'_, AppState>) -> Result<(), String> {
    // Two halves must go: rows already handed to earlier polls (the session) and rows the
    // capture buffered since the last poll — otherwise the next poll restores history.
    if let Ok(mut captures) = state.network_captures.lock() {
        if let Some(capture) = captures.get_mut(&id) {
            capture.clear();
        }
    }
    if let Ok(mut sessions) = state.network_sessions.lock() {
        if let Some(session) = sessions.get_mut(&id) {
            capture_flow(
                &format!("capture.clear.{id}"),
                "清空抓包请求",
                &format!("id={id} rows={} source={}", session.requests.len(), session.source),
            );
            session.requests.clear();
        }
    }
    Ok(())
}

#[tauri::command]
pub fn stop_network_capture(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut stop_error = None;
    if let Ok(mut captures) = state.network_captures.lock() {
        if let Some(mut capture) = captures.remove(&id) {
            let channel = if capture.is_proxy() { "proxy" } else { "cdp" };
            capture_flow(
                &format!("capture.stop.{id}"),
                "停止抓包",
                &format!("id={id} source={channel} serial={}", capture.serial()),
            );
            if let Err(e) = capture.stop() {
                stop_error = Some(e);
            }
        }
    }
    if let Ok(mut sessions) = state.network_sessions.lock() {
        if let Some(session) = sessions.get_mut(&id) {
            session.streaming = false;
        }
    }
    match stop_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn close_network_capture(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut stop_error = None;
    if let Ok(mut captures) = state.network_captures.lock() {
        if let Some(mut capture) = captures.remove(&id) {
            let channel = if capture.is_proxy() { "proxy" } else { "cdp" };
            capture_flow(
                &format!("capture.close.{id}"),
                "关闭抓包会话",
                &format!("id={id} source={channel} serial={}", capture.serial()),
            );
            if let Err(e) = capture.stop() {
                stop_error = Some(e);
            }
        }
    }
    if let Ok(mut sessions) = state.network_sessions.lock() {
        sessions.remove(&id);
    }
    match stop_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Import a HAR 1.2 file (Charles / mitmproxy / DevTools) or our own JSONL capture.
#[tauri::command]
pub fn import_network_capture(
    path: String,
    state: State<'_, AppState>,
) -> Result<NetworkSessionInfo, String> {
    let file_path = Path::new(&path);
    if !file_path.is_file() {
        return Err("所选路径不是文件".to_string());
    }
    let text = fs::read_to_string(file_path).map_err(|e| format!("读取抓包文件失败: {e}"))?;
    let mut requests = network::parse_capture_file(&text)?;
    for (i, request) in requests.iter_mut().enumerate() {
        request.index = i as u32;
    }
    let name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let id = format!("net-{}", state.next_id());
    let session = NetworkSession {
        requests,
        device_ip: String::new(),
        label: name,
        source: "file".to_string(),
        device: String::new(),
        streaming: false,
    };
    let info = network_info(&id, &session);
    capture_flow(
        "capture.file",
        "抓包文件已导入",
        &format!("id={id} rows={}", session.requests.len()),
    );
    state
        .network_sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?
        .insert(id, session);
    Ok(info)
}

/// Set the device-wide HTTP proxy for the MITM route. The guard is dropped (which clears
/// the device proxy) on `restore_device_proxy`, `close`, or app exit.
#[tauri::command]
pub fn configure_device_proxy(
    serial: String,
    proxy: String,
    state: State<'_, AppState>,
) -> Result<ProxyState, String> {
    let serial = serial.trim().to_string();
    let adb_path = require_adb()?;
    let mut guards = state
        .proxy_guards
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    if let Some(mut previous) = guards.remove(&serial) {
        previous.restore()?;
    }
    let guard = ProxyGuard::apply(&adb_path, &serial, &proxy)?;
    let applied = ProxyState {
        serial: serial.clone(),
        proxy: guard.applied().to_string(),
        original: guard.original().to_string(),
        active: true,
    };
    guards.insert(serial, guard);
    Ok(applied)
}

#[tauri::command]
pub fn restore_device_proxy(
    serial: String,
    state: State<'_, AppState>,
) -> Result<ProxyState, String> {
    let serial = serial.trim().to_string();
    let adb_path = require_adb()?;
    let mut had_guard = false;
    {
        let mut guards = state
            .proxy_guards
            .lock()
            .map_err(|e| format!("状态锁异常: {e}"))?;
        if let Some(mut guard) = guards.remove(&serial) {
            had_guard = true;
            guard.restore()?;
        }
    }
    if had_guard {
        return Ok(ProxyState {
            serial,
            ..Default::default()
        });
    }

    // No in-memory guard: this is the "app was killed / crashed / cable pulled" case.
    // Clear the device proxy unconditionally — keeping the previous value is what left a
    // dead endpoint behind in the first place. When the device is gone the restore fails,
    // and the journal entry must survive so the retry thread / next-start heal can still
    // finish the job once it is back.
    //
    // Run the restore even when Settings reads clean: on Android 15 the effective proxy
    // can survive a clean-looking `settings delete`, and only an explicit `:0` makes the
    // system stop handing it to newly started apps.
    if let Err(e) = netcapture::restore_proxy_settings(&adb_path, &serial) {
        // Journal entry stays; retry once the device is reachable again.
        netcapture::schedule_proxy_restore_retry(&adb_path, &serial);
        return Err(e);
    }
    netcapture::clear_proxy_marker(&serial);
    Ok(ProxyState {
        serial,
        ..Default::default()
    })
}

/* ------------------------------------------------- standalone proxy ------ */

/// Open a local proxy that is not tied to any device.
///
/// This is the proxy half on its own: any client that is pointed at the printed endpoint
/// (another phone, an emulator, an app with a manually configured proxy) is captured,
/// grouped by source address. No adb is involved, and no device setting is touched —
/// "start native capture" is the automation that does the configuring.
#[tauri::command]
pub fn start_proxy_server(
    port: u16,
    state: State<'_, AppState>,
) -> Result<crate::proxy::ProxyServerStatus, String> {
    // `0` means "use the default", exactly like `start_proxy_capture`. Passing it straight
    // through would bind an OS-chosen ephemeral port, so the address a user copies would
    // change on every restart.
    let port = if port == 0 { PROXY_PORT } else { port };
    let mut runtime = state
        .proxy_runtime
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    // Idempotent: if a device capture already brought the listener up, the same endpoint is
    // returned instead of failing — and that endpoint is now the *only* one in the process.
    let endpoint = runtime.ensure_listener(port)?;
    crate::error_log::record_flow(
        "proxy.server",
        "独立代理已开启",
        &format!("endpoint={endpoint} devices={}", runtime.devices().len()),
    );
    Ok(proxy_status(&runtime))
}

/// Leave the standalone view.
///
/// The listener is shared with device captures, so it is only closed when nothing else uses
/// it: with devices still bound, stopping here just clears the unbound-source groups and the
/// proxy keeps serving those devices. That is the difference between "close this page" and
/// "take the phone offline".
#[tauri::command]
pub fn stop_proxy_server(state: State<'_, AppState>) -> Result<(), String> {
    let mut runtime = state
        .proxy_runtime
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    let devices = runtime.devices().len();
    runtime.clear_unnamed();
    if devices == 0 {
        runtime.shutdown();
    }
    crate::error_log::record_flow(
        "proxy.server",
        "独立代理已关闭",
        &format!("devices={devices} listener_stopped={}", devices == 0),
    );
    Ok(())
}

/// Status of the shared proxy, including the endpoint clients must be pointed at and how
/// many devices are bound to the same listener.
#[tauri::command]
pub fn proxy_server_status(
    state: State<'_, AppState>,
) -> Result<crate::proxy::ProxyServerStatus, String> {
    let runtime = state
        .proxy_runtime
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    Ok(proxy_status(&runtime))
}

/// Everything captured by sources that are not a bound device, i.e. the standalone page.
#[tauri::command]
pub fn poll_proxy_server(
    state: State<'_, AppState>,
) -> Result<Vec<crate::network::NetworkRequest>, String> {
    let mut runtime = state
        .proxy_runtime
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    Ok(runtime.drain_unnamed())
}

/* --------------------------------------------------- client error log ------ */

/// Newest-first view of this app's own failures. No `AppHandle` needed: `error_log`
/// resolves its file once during `.setup()` and keeps it in process state.
#[tauri::command]
pub fn list_error_log() -> Result<Vec<crate::model::ErrorRecord>, String> {
    Ok(crate::error_log::list())
}

#[tauri::command]
pub fn clear_error_log() -> Result<(), String> {
    crate::error_log::clear()
}

#[tauri::command]
pub fn error_log_location() -> Result<String, String> {
    crate::error_log::location()
}

/// Called by the webview's global error handlers and by the IPC wrapper. Failures here
/// are best-effort: a broken diagnostics channel must never mask the original error.
#[tauri::command]
pub fn report_client_error(
    level: String,
    source: String,
    scope: String,
    message: String,
    detail: String,
) {
    crate::error_log::record(&level, &source, &scope, &message, &detail);
}

#[tauri::command]
pub fn export_error_log(dest_path: String) -> Result<usize, String> {
    let text = crate::error_log::export_text();
    fs::write(&dest_path, &text).map_err(|e| format!("写入错误日志文件失败: {e}"))?;
    Ok(text.len())
}

// --- search keyword history (persisted convenience state) ---

#[tauri::command]
pub fn list_search_history(app: tauri::AppHandle) -> Vec<SearchHistoryEntry> {
    config_store::load_search_history(&app)
}

#[tauri::command]
pub fn remember_search(
    app: tauri::AppHandle,
    keyword: String,
    mode: String,
    is_regex: bool,
    case_sensitive: bool,
) -> Result<Vec<SearchHistoryEntry>, String> {
    config_store::remember_search(&app, &keyword, &mode, is_regex, case_sensitive)
}

#[tauri::command]
pub fn delete_search_history(
    app: tauri::AppHandle,
    index: usize,
) -> Result<Vec<SearchHistoryEntry>, String> {
    config_store::delete_search_history(&app, index)
}

#[tauri::command]
pub fn clear_search_history(app: tauri::AppHandle) -> Result<Vec<SearchHistoryEntry>, String> {
    config_store::clear_search_history(&app)
}
