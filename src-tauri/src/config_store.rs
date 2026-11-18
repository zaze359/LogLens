//! Persistent user-defined rules and rule categories, stored as JSON in the OS
//! application config directory.
//! macOS: ~/Library/Application Support/com.zaze.loglens/
//! Windows: %APPDATA%/com.zaze.loglens/
//!
//! On-disk format (v2):
//! ```json
//! { "version": 2, "categories": [ ... ], "rules": [ ... ] }
//! ```
//! `parse_document` also accepts the v1 object (`{ "version": 1, "rules": [...] }`)
//! and the bare rule array produced by older exports / imports, so upgrading the app
//! never invalidates a user's existing rules file.

use crate::builtin;
use crate::model::{Rule, RuleCategory, SearchHistoryEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

pub const RULES_FILE_VERSION: u8 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RulesFile {
    pub version: u8,
    /// Only user-defined categories are persisted; built-in ones come from templates.json.
    pub categories: Vec<RuleCategory>,
    pub rules: Vec<Rule>,
}

fn rules_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法定位配置目录: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录 {}: {e}", dir.display()))?;
    Ok(dir.join("rules.json"))
}

/// Search history lives in its own file so a corrupt history never blocks rule loading
/// (and vice versa). Same config directory, same backup story as `rules.json`.
fn search_history_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法定位配置目录: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录 {}: {e}", dir.display()))?;
    Ok(dir.join("search-history.json"))
}

/// Cap so a long-lived install cannot grow the file without bound.
const SEARCH_HISTORY_LIMIT: usize = 50;

/// What the frontend needs: built-in + user categories together with custom rules.
/// Built-in templates are fetched separately (`list_builtin_templates`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RulesView {
    pub categories: Vec<RuleCategory>,
    pub rules: Vec<Rule>,
}

pub fn view(file: &RulesFile) -> RulesView {
    let mut categories = builtin::categories();
    categories.extend(file.categories.iter().cloned());
    RulesView {
        categories,
        rules: file.rules.clone(),
    }
}

/// Tolerant parser: v2 document, v1 document, or a bare `Vec<Rule>` array.
pub fn parse_document(text: &str) -> Result<RulesFile, String> {
    if text.trim().is_empty() {
        return Ok(RulesFile::default());
    }
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("规则文件格式错误: {e}"))?;
    if value.is_array() {
        let rules = serde_json::from_value(value)
            .map_err(|e| format!("规则文件格式错误: {e}"))?;
        return Ok(RulesFile {
            version: 1,
            categories: Vec::new(),
            rules,
        });
    }
    if !value.is_object() {
        return Err("规则文件格式错误：应为对象或规则数组".to_string());
    }
    // Read fields individually so a missing / malformed optional field never aborts
    // loading the rules that are still valid.
    Ok(RulesFile {
        version: value
            .get("version")
            .and_then(Value::as_u64)
            .unwrap_or(1) as u8,
        categories: value
            .get("categories")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        rules: value
            .get("rules")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
    })
}

pub fn builtin_category_ids() -> HashSet<String> {
    builtin::categories()
        .into_iter()
        .map(|c| c.id)
        .collect::<HashSet<_>>()
}

/// Drop anything that must not be user-editable, de-duplicate ids, and repoint rules
/// whose category was deleted (or never existed) to the uncategorised bucket.
fn normalise(mut file: RulesFile) -> RulesFile {
    let builtin_ids = builtin_category_ids();
    file.rules
        .retain(|r| !r.builtin && !r.id.starts_with("builtin-") && !r.id.trim().is_empty());
    file.categories.retain(|c| {
        !c.builtin && !builtin_ids.contains(&c.id) && !c.id.trim().is_empty()
    });

    let mut seen: HashSet<String> = HashSet::new();
    file.categories.retain(|c| seen.insert(c.id.clone()));

    let mut valid_ids = builtin_ids;
    valid_ids.extend(file.categories.iter().map(|c| c.id.clone()));
    for rule in &mut file.rules {
        if !rule.category_id.is_empty() && !valid_ids.contains(&rule.category_id) {
            rule.category_id.clear();
        }
    }
    file
}

pub fn load(app: &AppHandle) -> Result<RulesFile, String> {
    let path = rules_path(app)?;
    if !path.exists() {
        return Ok(RulesFile {
            version: RULES_FILE_VERSION,
            ..Default::default()
        });
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("读取规则失败: {e}"))?;
    Ok(normalise(parse_document(&text)?))
}

pub fn save(app: &AppHandle, file: &RulesFile) -> Result<RulesFile, String> {
    let path = rules_path(app)?;
    let payload = RulesFile {
        version: RULES_FILE_VERSION,
        ..normalise(file.clone())
    };
    let text =
        serde_json::to_string_pretty(&payload).map_err(|e| format!("序列化规则失败: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入规则失败 {}: {e}", path.display()))?;
    Ok(payload)
}

/// Replace only the rules, keeping user categories intact.
pub fn save_rules(app: &AppHandle, rules: &[Rule]) -> Result<RulesView, String> {
    let mut file = load(app)?;
    file.rules = rules.to_vec();
    Ok(view(&save(app, &file)?))
}

/// Replace only the user-defined categories, keeping rules intact.
pub fn save_categories(app: &AppHandle, categories: &[RuleCategory]) -> Result<RulesView, String> {
    let mut file = load(app)?;
    file.categories = categories.to_vec();
    Ok(view(&save(app, &file)?))
}

/// Absolute path of the rules file, shown in the UI so users can back it up manually.
pub fn location(app: &AppHandle) -> Result<String, String> {
    rules_path(app).map(|p| p.to_string_lossy().into_owned())
}

/// Load remembered searches, newest first. Tolerant: a missing or malformed file is an
/// empty history, not a startup failure — this is a convenience feature, not app state.
pub fn load_search_history(app: &AppHandle) -> Vec<SearchHistoryEntry> {
    let path = match search_history_path(app) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    normalise_history(serde_json::from_str(&text).unwrap_or_default())
}

/// Pure list hygiene, shared by load / remember and unit-tested without an AppHandle.
fn normalise_history(mut entries: Vec<SearchHistoryEntry>) -> Vec<SearchHistoryEntry> {
    entries.retain(|e| !e.keyword.trim().is_empty());
    if entries.len() > SEARCH_HISTORY_LIMIT {
        entries.truncate(SEARCH_HISTORY_LIMIT);
    }
    entries
}

fn write_search_history(app: &AppHandle, entries: &[SearchHistoryEntry]) -> Result<(), String> {
    let path = search_history_path(app)?;
    let text = serde_json::to_string_pretty(entries)
        .map_err(|e| format!("序列化搜索历史失败: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入搜索历史失败 {}: {e}", path.display()))
}

/// Remember a keyword. De-duplicated on (keyword, mode, flags): reusing an existing search
/// bumps it to the front with a fresh timestamp instead of adding a twin. Returns the new
/// full list so the caller can update the UI in one round trip.
pub fn remember_search(
    app: &AppHandle,
    keyword: &str,
    mode: &str,
    is_regex: bool,
    case_sensitive: bool,
) -> Result<Vec<SearchHistoryEntry>, String> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(load_search_history(app));
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut entries = load_search_history(app);
    entries.retain(|e| {
        !(e.keyword == keyword
            && e.mode == mode
            && e.is_regex == is_regex
            && e.case_sensitive == case_sensitive)
    });
    entries.insert(
        0,
        SearchHistoryEntry {
            keyword: keyword.to_string(),
            mode: mode.to_string(),
            is_regex,
            case_sensitive,
            ts,
        },
    );
    if entries.len() > SEARCH_HISTORY_LIMIT {
        entries.truncate(SEARCH_HISTORY_LIMIT);
    }
    write_search_history(app, &entries)?;
    Ok(entries)
}

/// Remove one entry by index as sent by the UI (it renders the same list it receives).
pub fn delete_search_history(
    app: &AppHandle,
    index: usize,
) -> Result<Vec<SearchHistoryEntry>, String> {
    let mut entries = load_search_history(app);
    if index < entries.len() {
        entries.remove(index);
        write_search_history(app, &entries)?;
    }
    Ok(entries)
}

pub fn clear_search_history(app: &AppHandle) -> Result<Vec<SearchHistoryEntry>, String> {
    write_search_history(app, &[])?;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn condition_rule(id: &str, category: &str) -> Rule {
        Rule {
            id: id.to_string(),
            name: format!("rule {id}"),
            category_id: category.to_string(),
            group: crate::model::RuleGroup {
                logic: "AND".to_string(),
                conditions: vec![crate::model::Condition {
                    id: "c1".to_string(),
                    field: "message".to_string(),
                    op: "contains".to_string(),
                    value: "boom".to_string(),
                    case_sensitive: false,
                    negate: false,
                }],
            },
            ..Default::default()
        }
    }

    #[test]
    fn parses_v2_document() {
        // `"package"` is deliberately still present: files written by the earlier build
        // carried it on categories, and serde must silently ignore the stale key so an
        // existing rules.json keeps loading after the field was removed.
        let text = r#"{
            "version": 2,
            "categories": [{"id":"custom-cat-a","name":"App A","package":"com.a"}],
            "rules": [{"id":"r1","name":"r","categoryId":"custom-cat-a","group":{"logic":"AND","conditions":[]}}]
        }"#;
        let file = parse_document(text).unwrap();
        assert_eq!(file.version, 2);
        assert_eq!(file.categories.len(), 1);
        assert_eq!(file.categories[0].name, "App A");
        assert_eq!(file.rules.len(), 1);
        assert_eq!(file.rules[0].category_id, "custom-cat-a");
    }

    #[test]
    fn parses_v1_document_and_bare_array() {
        let v1 = r#"{"version":1,"rules":[{"id":"r1","name":"old"}]}"#;
        let file = parse_document(v1).unwrap();
        assert_eq!(file.version, 1);
        assert!(file.categories.is_empty());
        assert_eq!(file.rules.len(), 1);

        let arr = r#"[{"id":"r2","name":"exported"}]"#;
        let file = parse_document(arr).unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.rules.len(), 1);
        assert_eq!(file.rules[0].id, "r2");
    }

    #[test]
    fn empty_or_invalid_documents_are_handled() {
        assert!(parse_document("  ").unwrap().rules.is_empty());
        assert_eq!(parse_document(r#"{"rules":null}"#).unwrap().rules.len(), 0);
        assert!(parse_document("42").is_err());
    }

    #[test]
    fn view_merges_builtin_and_user_categories() {
        let file = RulesFile {
            version: 2,
            categories: vec![RuleCategory {
                id: "custom-cat-a".to_string(),
                name: "App A".to_string(),
                ..Default::default()
            }],
            rules: vec![condition_rule("r1", "custom-cat-a")],
        };
        let out = view(&file);
        assert_eq!(out.rules.len(), 1);
        assert!(out.categories.iter().any(|c| c.builtin));
        assert!(out.categories.iter().any(|c| c.id == "custom-cat-a" && !c.builtin));
    }

    #[test]
    fn search_history_drops_blanks_and_caps_length() {
        let entries: Vec<SearchHistoryEntry> = (0..(SEARCH_HISTORY_LIMIT + 10))
            .map(|i| SearchHistoryEntry {
                keyword: format!("kw-{i}"),
                ..Default::default()
            })
            .chain(std::iter::once(SearchHistoryEntry {
                keyword: "   ".to_string(),
                ..Default::default()
            }))
            .collect();
        let out = normalise_history(entries);
        assert_eq!(out.len(), SEARCH_HISTORY_LIMIT);
        assert_eq!(out[0].keyword, "kw-0");
        assert!(out.iter().all(|e| !e.keyword.trim().is_empty()));
    }

    #[test]
    fn search_history_malformed_text_is_an_empty_list() {
        // Convenience state must never abort startup on a corrupt/truncated file.
        let bad: Vec<SearchHistoryEntry> = serde_json::from_str("{not json").unwrap_or_default();
        assert!(normalise_history(bad).is_empty());
    }

    #[test]
    fn normalise_drops_builtins_and_repairs_dangling_categories() {
        let file = RulesFile {
            version: 1,
            categories: vec![
                RuleCategory {
                    id: "custom-cat-a".to_string(),
                    name: "App A".to_string(),
                    ..Default::default()
                },
                RuleCategory {
                    id: "custom-cat-a".to_string(),
                    name: "duplicate".to_string(),
                    ..Default::default()
                },
                RuleCategory {
                    id: "cat-general".to_string(),
                    builtin: true,
                    ..Default::default()
                },
            ],
            rules: vec![
                condition_rule("r1", "custom-cat-a"),
                condition_rule("r2", "deleted-cat"),
                condition_rule("builtin-errors", "cat-general"),
            ],
        };
        let out = normalise(file);
        assert_eq!(out.categories.len(), 1, "duplicate and builtin categories removed");
        assert_eq!(out.rules.len(), 2, "builtin rule removed");
        assert_eq!(out.rules[0].category_id, "custom-cat-a");
        assert_eq!(out.rules[1].category_id, "", "dangling category cleared");
    }
}
