//! Wire types shared between the Rust backend and the Vue frontend.
//! Every struct uses `camelCase` on the JSON boundary and tolerates missing fields,
//! so the frontend can evolve without breaking the IPC contract.

use crate::netcapture::NetworkCaptureStatus;
use crate::network::NetworkRequest;
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
    /// Package reconstructed from the pid (`ps` snapshot + `am_proc_start` /
    /// `Start proc` rows), or lifted out of an accessibility foreground event. Empty when
    /// the pid is unknown or belongs to a non-application process.
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
    /// Application packages (or the package part of a sub-process). Any of them matches.
    pub packages: Vec<String>,
    pub search: String,
    /// What `search` means: `filter` (default) narrows the result set like every other
    /// quick filter; `find` is a *search* — it leaves the result set untouched and the hits
    /// come from `search_hits`, which the UI turns into "3 / 27" plus jump-to-next.
    ///
    /// The distinction matters because the two are used differently: filtering is "only show
    /// me what matches", finding is "where does this string appear in what I am already
    /// looking at". Export follows the filter, so `find` never changes what gets written out.
    pub search_mode: String,
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
    /// Where the entries came from: `file` (imported log file) or `stream` (live device stream).
    pub source: String,
    /// Device serial for the `stream` source, empty for files.
    pub device: String,
    /// True while a live `adb logcat` stream is still appending to this session.
    pub streaming: bool,
}

/// Options for attaching a live `adb logcat` stream to a connected device.
///
/// There is deliberately only one capture mode: a live stream that keeps appending
/// until the user stops it. A one-shot dump looks identical in the UI and then silently
/// freezes, which reads as "the app stopped updating" — and `adb logcat` already replays
/// the existing buffer on start, so a live stream also covers "look at what already
/// happened".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceCaptureOptions {
    pub serial: String,
    /// `main` / `system` / `crash` / `events` / `all`. Empty = adb's default buffers.
    pub buffers: Vec<String>,
    /// Run `adb logcat -c` first, so the stream starts from a clean buffer. Default off:
    /// wiping the device buffer destroys evidence and cannot be undone.
    pub clear_before: bool,
}

/// Health of a live capture, polled by the UI while `streaming` is true.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamStatus {
    pub session_id: String,
    pub running: bool,
    /// Entries already parsed into the session (including the ones appended by this poll).
    pub entry_count: u32,
    pub total_lines: u32,
    pub unparsed_lines: u32,
    /// Non-empty when the stream ended abnormally (`adb` error, device unplugged, ...).
    pub error: String,
    /// True when this poll appended entries, in which case the facet lists below are fresh.
    /// The UI keeps its previous lists while this is false instead of clearing the filters.
    pub facets_updated: bool,
    pub levels: Vec<String>,
    pub tags: Vec<String>,
    pub pids: Vec<String>,
    pub packages: Vec<String>,
}

/// Result of a `find` query: where the search term appears inside the *filtered* set.
///
/// `total` counts every hit while `indices` is capped (a term like `e` can match hundreds of
/// thousands of entries; the UI only needs enough to jump around, and `truncated` says the
/// exact counts stop here).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchHits {
    pub total: u32,
    /// Entry indices, in display order, capped at the requested limit.
    pub indices: Vec<u32>,
    pub truncated: bool,
}

/// Progress of a file import, emitted as `log-open-progress` while `open_log_file` runs.
///
/// Carries `requestId` because the frontend may have several imports in flight and must only
/// apply the events belonging to the one it is displaying. `totalBytes` is 0 until the file
/// has been stat'ed, which the UI treats as "size unknown, show an indeterminate bar".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenProgress {
    pub request_id: String,
    pub bytes_read: u64,
    pub total_bytes: u64,
    /// Entries parsed so far (not lines): one entry may cover a whole stack trace.
    pub entries: u32,
    /// `parsing` while reading the file, `indexing` while building facet lists afterwards.
    /// The second phase is what keeps the bar from sitting at 100% with no explanation.
    pub phase: String,
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

// --- Device network capture -------------------------------------------------
//
// Traffic sessions are deliberately kept separate from log sessions: a request has
// method / status / duration columns, not tag / level / pid. The two timelines share
// `time_key` (`HH:MM:SS.mmm`) so the UI can correlate them later.

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CdpCaptureOptions {
    pub serial: String,
    /// `webview_devtools_remote_<pid>` from the capability probe.
    pub socket: String,
    /// Local TCP port for `adb forward`. 0 = pick the default.
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkSessionInfo {
    pub id: String,
    pub label: String,
    /// `cdp` | `file`
    pub source: String,
    pub device: String,
    pub streaming: bool,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkPollResult {
    pub id: String,
    /// Full current list. Captures are short and bounded, so a full snapshot is cheaper
    /// and far less error-prone than reconciling incremental indices in the UI.
    pub requests: Vec<NetworkRequest>,
    pub status: NetworkCaptureStatus,
    pub added: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxyState {
    pub serial: String,
    pub proxy: String,
    /// Value that will be put back on restore; empty means "delete the setting".
    pub original: String,
    pub active: bool,
}
