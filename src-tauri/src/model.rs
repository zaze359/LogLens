//! Wire types shared between the Rust backend and the Vue frontend.
//! Every struct uses `camelCase` on the JSON boundary and tolerates missing fields,
//! so the frontend can evolve without breaking the IPC contract.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LogEntry {
    /// Position of this entry inside the parsed file (0-based).
    pub index: u32,
    /// Original first line, plus continuation lines joined by `\n`. Loss-free for export.
    pub raw: String,
    /// Display timestamp, e.g. `11-12 10:23:45.123` or `2026-11-12 10:23:45.123`.
    pub timestamp: String,
    /// Normalised `HH:MM:SS.mmm`, used for time range comparisons.
    pub time_key: String,
    /// `V D I W E F A S ?` (`?` = line could not be parsed).
    pub level: String,
    pub tag: String,
    pub pid: u32,
    pub tid: u32,
    pub package: String,
    /// `AccessibilityEvent.className` extracted from foreground-app events such as
    /// `onTopAppChanged: <packageName>/<className>`. The part before `/` is the
    /// `packageName` and lands in `package`; className may be an Activity, a widget
    /// (`android.widget.FrameLayout`), a dialog, etc. Empty for ordinary log lines.
    pub component: String,
    /// Message plus merged continuation lines (stack traces, long payloads).
    pub message: String,
    /// Number of continuation lines merged into this entry.
    pub continuation: u32,
    pub parse_ok: bool,
}

/// A single structured condition, e.g. `tag regex ^OkHttp`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Condition {
    pub id: String,
    /// level | tag | message | raw | pid | tid | package | timestamp | timeKey
    pub field: String,
    /// contains | notContains | equals | notEquals | startsWith | endsWith | regex | in | gte | lte
    pub op: String,
    pub value: String,
    pub case_sensitive: bool,
    pub negate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RuleGroup {
    /// `AND` or `OR`
    pub logic: String,
    pub conditions: Vec<Condition>,
}

/// A named bucket of rules — pure classification, deliberately **not** bound to any
/// application. Built-in categories group the shipped templates by topic; users can add
/// their own categories (per business module, per app, per problem type, ...).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RuleCategory {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Accent colour used by the category header (CSS colour string).
    pub color: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Accent colour used by the sidebar chip (CSS colour string).
    pub color: String,
    pub builtin: bool,
    /// Id of the owning `RuleCategory`. Empty string = uncategorised. Named `categoryId`
    /// rather than `group` because `RuleGroup` already means a set of conditions.
    pub category_id: String,
    /// Structured conditions (AND/OR). Rule matching itself is unaffected by categories.
    pub group: RuleGroup,
}

/// All quick filters plus an optional structured rule group.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterSpec {
    pub levels: Vec<String>,
    pub tags: Vec<String>,
    pub pids: Vec<String>,
    pub search: String,
    pub is_regex: bool,
    pub case_sensitive: bool,
    /// Comma separated keywords; any hit removes the entry.
    pub exclude: String,
    /// `HH:MM:SS` (optionally with `.mmm`) — inclusive.
    pub time_from: String,
    pub time_to: String,
    pub group: Option<RuleGroup>,
    /// How the quick filters combine with `group`: `AND` (default) or `OR`.
    pub group_logic: String,
    /// `asc` | `desc`
    pub order: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NameCount {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FileInfo {
    pub session_id: String,
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub total_lines: u32,
    pub entry_count: u32,
    pub unparsed_lines: u32,
    pub levels: Vec<String>,
    pub tags: Vec<String>,
    pub pids: Vec<String>,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct QueryResult {
    /// Total entries in the file (before filtering).
    pub total: u32,
    /// Entries matching the filter.
    pub matched: u32,
    pub entries: Vec<LogEntry>,
    /// Level distribution of matched entries.
    pub level_counts: Vec<NameCount>,
    /// Top 10 tags of matched entries.
    pub top_tags: Vec<NameCount>,
    pub elapsed_ms: u64,
}
