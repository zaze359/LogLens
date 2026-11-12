//! Tauri IPC commands. All file IO happens here in Rust, so no filesystem plugin scope
//! has to be configured for arbitrary user-selected log files.

use crate::builtin;
use crate::config_store::{self, RulesView};
use crate::logcat::{self, LogcatParser};
use crate::model::{FileInfo, FilterSpec, QueryResult, Rule, RuleCategory};
use crate::rules::CompiledFilter;
use crate::session::{AppState, LogSession};
use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};

const MAX_OPEN_SESSIONS: usize = 4;

#[tauri::command]
pub fn open_log_file(path: String, state: State<'_, AppState>) -> Result<FileInfo, String> {
    let file_path = Path::new(&path);
    let meta = fs::metadata(file_path).map_err(|e| format!("无法读取文件信息: {e}"))?;
    if !meta.is_file() {
        return Err("所选路径不是文件".to_string());
    }
    let parsed = LogcatParser::parse_file(file_path)?;
    let (levels, tags, pids, packages) = logcat::collect_facets(&parsed.entries);

    let session_id = state.next_id();
    let info = FileInfo {
        session_id: session_id.clone(),
        path: path.clone(),
        file_name: file_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone()),
        size_bytes: meta.len(),
        total_lines: parsed.total_lines,
        entry_count: parsed.entries.len() as u32,
        unparsed_lines: parsed.unparsed_lines,
        levels,
        tags,
        pids,
        packages,
    };

    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| format!("状态锁异常: {e}"))?;
    if sessions.len() >= MAX_OPEN_SESSIONS {
        if let Some(oldest) = sessions.keys().next().cloned() {
            sessions.remove(&oldest);
        }
    }
    sessions.insert(session_id, LogSession::new(parsed.entries));
    Ok(info)
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
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
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
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
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
