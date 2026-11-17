//! Android Logcat parser.
//!
//! Supported `adb logcat -v` formats:
//!   threadtime (`PID TID L Tag: msg` — what a real device prints — plus the older
//!   `PID-TID/package L/Tag:` and uid variants, with/without year),
//!   time, brief, tag, process, long
//!
//! Design decisions:
//! - Parsing is deliberately tolerant: an unrecognised line is never dropped.
//! - Indented lines (stack frames, `Caused by`, JSON payloads, ...) are merged into the
//!   previous entry so a filter hit keeps its stack trace context.
//! - `--------- beginning of ...` becomes a synthetic `S/logcat` entry instead of
//!   polluting the previous message.
//! - `-v threadtime` carries no package column, so the package is reconstructed from the
//!   pid — see [`ProcessIndex`].

use crate::model::LogEntry;
use regex::{Captures, Regex};
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct ParseResult {
    pub entries: Vec<LogEntry>,
    /// Physical lines read from the file, including continuation lines.
    pub total_lines: u32,
    /// Standalone lines that could not be parsed into a known format.
    pub unparsed_lines: u32,
    /// pid → package, learned while parsing. Lives here (not in `LogcatParser`) so the
    /// parser itself stays immutable and shareable across concurrent sessions.
    pub processes: ProcessIndex,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormatKind {
    Dated,
    Plain,
    LongHeader,
    Process,
}

struct Format {
    re: Regex,
    kind: FormatKind,
}

/// Foreground-app events produced by an Android AccessibilityService. The part before
/// `/` is `AccessibilityEvent.packageName` (the app being observed); the part after is
/// `AccessibilityEvent.className`, which is not necessarily an Activity — it can be a
/// widget / dialog / popup such as `android.widget.FrameLayout`.
const ACCESSIBILITY_EVENT_RE: &str = r"(?i)(?:onTopAppChanged|topResumedActivity|mCurrentFocus|ResumedActivity|topActivity)\s*[:=]\s*(?P<pkg>[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z][A-Za-z0-9_]*)+)/(?P<component>[A-Za-z_$][A-Za-z0-9_.$]*)";

pub struct LogcatParser {
    formats: Vec<Format>,
    pair_dash_re: Regex,
    pair_space_re: Regex,
    pkg_re: Regex,
    tag_pid_re: Regex,
    divider_re: Regex,
    num_re: Regex,
    accessibility_re: Regex,
}

fn compile(pattern: &str) -> Result<Regex, String> {
    Regex::new(pattern).map_err(|e| format!("invalid logcat pattern: {e}"))
}

fn to_u32(caps: &Captures, name: &str) -> u32 {
    caps.name(name)
        .and_then(|m| m.as_str().parse::<u32>().ok())
        .unwrap_or(0)
}

/* ------------------------------------------------------------------ pid → package ---- */

fn re_am_proc_start() -> &'static Regex {
    // events: `am_proc_start: [userId, pid, uid, processName, type, component]`
    //   11-12 17:57:17.386 19484 19519 I am_proc_start: [0,13800,10174,com.foo,next-top-activity,{...}]
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\[\s*\d+\s*,\s*(?P<pid>\d+)\s*,\s*\d+\s*,\s*(?P<name>[A-Za-z0-9_.]+(?::[A-Za-z0-9_.]+)?)")
            .expect("valid am_proc_start pattern")
    })
}

fn re_am_proc_end() -> &'static Regex {
    // events: `am_proc_died: [userId, pid, processName, ...]`
    //         `am_kill:      [userId, pid, processName, oomAdj, reason]`
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\[\s*\d+\s*,\s*(?P<pid>\d+)\s*,\s*(?P<name>[A-Za-z0-9_.]+(?::[A-Za-z0-9_.]+)?)")
            .expect("valid am_proc_died pattern")
    })
}

fn re_start_proc() -> &'static Regex {
    // ActivityManager: `Start proc 13800:com.foo/u0a174 for next-top-activity {...}`
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"Start proc\s+(?P<pid>\d+):(?P<name>[A-Za-z0-9_.]+(?::[A-Za-z0-9_.]+)?)")
            .expect("valid Start proc pattern")
    })
}

fn re_killing() -> &'static Regex {
    // ActivityManager: `Killing 13122:com.foo/u0a174 (adj 200): stop com.foo`
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"Killing\s+(?P<pid>\d+):(?P<name>[A-Za-z0-9_.]+)").expect("valid Killing pattern")
    })
}

fn re_process_died() -> &'static Regex {
    // ActivityManager: `Process com.foo (pid 1234) has died`
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"Process\s+(?P<name>[A-Za-z0-9_.]+(?::[A-Za-z0-9_.]+)?)\s*\(pid\s+(?P<pid>\d+)\)\s*has died")
            .expect("valid has died pattern")
    })
}

fn re_pid_pkg() -> &'static Regex {
    // `<pid>:<package>/u0a174` — the shape ActivityManager / ProcessRecord dumps use
    // wherever they mention an app process. Requiring the `u<user>a<appid>` tail keeps this
    // from matching timestamps or random `12:34` text.
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?P<pid>\d{2,7}):(?P<name>[a-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+(?::[A-Za-z0-9_]+)?)/u\d+a\d+")
            .expect("valid pid:package pattern")
    })
}

/// `com.tencent.mm:push` → `com.tencent.mm`.
///
/// App sub-processes (push / remote / sandbox) are the same application, so filtering by a
/// package must also catch them. The exact pid is still available through the pid filter.
fn strip_subprocess(name: &str) -> String {
    match name.find(':') {
        Some(i) => name[..i].to_string(),
        None => name.to_string(),
    }
}

/// Whether a process name is an application package rather than a system process.
///
/// `adbd`, `logd`, `init`, `system_server` and vendor HAL services
/// (`vendor.lenovo.hardware.battery@1.0-service`) do run app code, but they have no
/// package. Keeping them out of `LogEntry.package` keeps the package filter about
/// applications; those logs stay reachable through tag / pid.
fn is_app_package(name: &str) -> bool {
    name.contains('.')
        && !name.starts_with("vendor.")
        && !name.starts_with("android.hardware")
        && !name.contains('@')
}

/// Split one `ps` row into `(pid, process name)`.
///
/// Handles both shapes seen in the wild:
/// - `ps -A -o PID,NAME` → `<pid> <name>` (what the app asks for: one name column, no spaces)
/// - classic `ps` → `<user> <pid> <ppid> ... <name>` (name is the last column)
///
/// Header rows fall through both branches because their second column is not a number.
fn parse_ps_row(line: &str) -> Option<(u32, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut tokens = trimmed.split_whitespace();
    let first = tokens.next()?;

    if let Ok(pid) = first.parse::<u32>() {
        let name = trimmed[first.len()..].trim();
        return clean_process_name(name).map(|name| (pid, name));
    }

    // Classic `ps`: the user column comes first, so the pid is the first numeric column.
    let mut rest = trimmed.split_whitespace();
    rest.next()?;
    let pid = rest.next()?.parse::<u32>().ok()?;
    let name = trimmed.split_whitespace().last()?;
    clean_process_name(name).map(|name| (pid, name))
}

fn clean_process_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    // `[kworker/0:1]` and friends are kernel threads, not applications.
    if name.is_empty() || name.starts_with('[') {
        return None;
    }
    Some(strip_subprocess(name))
}

/// pid → application package, reconstructed from the log stream itself.
///
/// `adb logcat -v threadtime` prints no package column, but the pid is always there and
/// three cheap sources identify its owner:
///
/// 1. **`ps` snapshots** (`adb shell ps -A -o PID,NAME`) taken when a capture starts and
///    refreshed while it runs. The process name of an app process *is* its package.
/// 2. **`events` buffer rows** — `am_proc_start: [user, pid, uid, package, ...]` and
///    `am_kill: [user, pid, package, ...]`. These stay in the ring buffer after the process
///    is gone, which is exactly the crash case where a live process listing cannot help.
/// 3. **`ActivityManager` text lines** — `Start proc 13800:com.foo/u0a174 for ...`,
///    `Killing 13122:com.foo/u0a174 (...)`, `Process com.foo (pid 1234) has died`.
///
/// Death rows remove the mapping so a recycled pid does not inherit the dead app's name.
#[derive(Debug, Default)]
pub struct ProcessIndex {
    by_pid: HashMap<u32, String>,
}

impl ProcessIndex {
    /// Adopt a `ps` snapshot. Returns how many rows were stored.
    pub fn merge_snapshot(&mut self, text: &str) -> usize {
        let mut adopted = 0;
        for line in text.lines() {
            if let Some((pid, name)) = parse_ps_row(line) {
                if pid > 0 {
                    self.by_pid.insert(pid, name);
                    adopted += 1;
                }
            }
        }
        adopted
    }

    /// Learn from one parsed entry, if it happens to name an app process.
    pub fn observe(&mut self, entry: &LogEntry) {
        let message = entry.message.as_str();
        match entry.tag.as_str() {
            "am_proc_start" => {
                if let Some(caps) = re_am_proc_start().captures(message) {
                    self.remember(&caps);
                }
                return;
            }
            "am_proc_died" | "am_kill" | "am_anr" | "am_crash" => {
                if let Some(caps) = re_am_proc_end().captures(message) {
                    self.forget(&caps);
                }
                return;
            }
            _ => {}
        }

        // Order matters: a `Killing 13122:com.foo/u0a174` row also matches the generic
        // `pid:package/uid` shape, so deaths have to be handled before learning.
        if let Some(caps) = re_killing().captures(message) {
            self.forget(&caps);
            return;
        }
        if let Some(caps) = re_process_died().captures(message) {
            self.forget(&caps);
            return;
        }
        if let Some(caps) = re_start_proc().captures(message) {
            self.remember(&caps);
            return;
        }
        if let Some(caps) = re_pid_pkg().captures(message) {
            self.remember(&caps);
        }
    }

    /// Fill `entry.package` when the line itself carried no package.
    ///
    /// A package spelled out in the line always wins: `1234-5678/com.foo` is authoritative,
    /// and an accessibility foreground event (`onTopAppChanged: com.example.launcher/...`)
    /// describes the app on screen, not the process that printed the line.
    pub fn fill(&self, entry: &mut LogEntry) {
        if !entry.package.is_empty() || entry.pid == 0 {
            return;
        }
        if let Some(name) = self.by_pid.get(&entry.pid) {
            if is_app_package(name) {
                entry.package = name.clone();
            }
        }
    }

    fn remember(&mut self, caps: &Captures<'_>) {
        let pid = to_u32(caps, "pid");
        let name = caps.name("name").map(|m| m.as_str()).unwrap_or("");
        if pid == 0 || name.is_empty() {
            return;
        }
        self.by_pid.insert(pid, strip_subprocess(name));
    }

    fn forget(&mut self, caps: &Captures<'_>) {
        let pid = to_u32(caps, "pid");
        if pid > 0 {
            self.by_pid.remove(&pid);
        }
    }
}

/// Normalise a time value to `HH:MM:SS.mmm` so string comparison works as a range filter.
/// Accepts `10:23`, `10:23:45`, `10:23:45.12`, `11-12 10:23:45,123`.
pub fn normalize_time_value(raw: &str) -> String {
    let cleaned = raw.trim().replace(',', ".");
    if cleaned.is_empty() {
        return String::new();
    }
    let tail = cleaned.rsplit(' ').next().unwrap_or(&cleaned).to_string();
    let mut parts = tail.splitn(2, '.');
    let base = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("");
    let base_fixed = match base.split(':').count() {
        1 => format!("{}:00:00", base),
        2 => format!("{}:00", base),
        _ => base.to_string(),
    };
    let mut millis: String = frac.chars().take(3).collect();
    while millis.len() < 3 {
        millis.push('0');
    }
    format!("{}.{}", base_fixed, millis)
}

impl LogcatParser {
    pub fn new() -> Result<Self, String> {
        let time = r"\d{2}:\d{2}:\d{2}(?:[.,]\d{1,6})?";
        let level = r"[VDIWEFASvdiwefas]";

        let mut formats = Vec::new();
        // `-v threadtime` on a real device has NO package column and NO slash:
        //   11-12 17:28:02.013  1080  1161 D PowerGuruService: message
        // These two must be tried before the slash variants: a message that itself contains
        // something like `E/http: 500` would otherwise be re-parsed as an entry whose tag is
        // `http`, silently moving the real tag into `middle`.
        formats.push(Format {
            re: compile(&format!(
                r"^(?P<date>\d{{4}}-\d{{2}}-\d{{2}}[ T])(?P<time>{time})\s+(?P<pid>\d{{1,7}})\s+(?P<tid>\d{{1,7}})\s+(?P<level>{level})\s+(?P<tag>[^:\s][^:]*?)\s*:\s?(?P<msg>.*)$"
            ))?,
            kind: FormatKind::Dated,
        });
        formats.push(Format {
            re: compile(&format!(
                r"^(?P<date>\d{{2}}-\d{{2}})\s+(?P<time>{time})\s+(?P<pid>\d{{1,7}})\s+(?P<tid>\d{{1,7}})\s+(?P<level>{level})\s+(?P<tag>[^:\s][^:]*?)\s*:\s?(?P<msg>.*)$"
            ))?,
            kind: FormatKind::Dated,
        });
        // 2026-11-12 10:23:45.123  1234-5678/com.example D/Tag: message
        formats.push(Format {
            re: compile(&format!(
                r"^(?P<date>\d{{4}}-\d{{2}}-\d{{2}}[ T])(?P<time>{time})\s+(?P<middle>.*?)\s*(?P<level>{level})/(?P<tag>[^:]*?)\s*:\s?(?P<msg>.*)$"
            ))?,
            kind: FormatKind::Dated,
        });
        // 11-12 10:23:45.123  1234-5678/com.example D/Tag: message
        // also covers `-v time`: 11-12 10:23:45.123 D/Tag( 1234): message
        formats.push(Format {
            re: compile(&format!(
                r"^(?P<date>\d{{2}}-\d{{2}})\s+(?P<time>{time})\s+(?P<middle>.*?)\s*(?P<level>{level})/(?P<tag>[^:]*?)\s*:\s?(?P<msg>.*)$"
            ))?,
            kind: FormatKind::Dated,
        });
        // [ 11-12 10:23:45.123  1234:5678 D/Tag ]
        formats.push(Format {
            re: compile(&format!(
                r"^\[\s*(?:(?P<date>\d{{2}}-\d{{2}})\s+)?(?P<time>{time})\s+(?P<middle>[^\]]*?)\s*(?P<level>{level})/(?P<tag>[^\]]*?)\s*\]\s*$"
            ))?,
            kind: FormatKind::LongHeader,
        });
        // D( 1234) message (Tag)
        formats.push(Format {
            re: compile(&format!(
                r"^(?P<level>{level})\(\s*(?P<pid>\d+)\)\s+(?P<msg>.*?)\s+\((?P<tag>[^)]*)\)\s*$"
            ))?,
            kind: FormatKind::Process,
        });
        // D/Tag: message  /  D/Tag( 1234): message
        formats.push(Format {
            re: compile(&format!(
                r"^(?P<level>{level})/(?P<tag>[^:]*?)\s*:\s?(?P<msg>.*)$"
            ))?,
            kind: FormatKind::Plain,
        });

        Ok(Self {
            formats,
            pair_dash_re: compile(r"(?P<pid>\d+)\s*[-:]\s*(?P<tid>\d+)")?,
            pair_space_re: compile(r"(?P<pid>\d+)\s+(?P<tid>\d+)")?,
            pkg_re: compile(r"/(?P<pkg>[A-Za-z_][A-Za-z0-9_.:-]*)")?,
            tag_pid_re: compile(r"\(\s*(\d+)\s*\)")?,
            divider_re: compile(r"^-{5,}\s*beginning of\s+(?P<label>.+)$")?,
            num_re: compile(r"\d+")?,
            accessibility_re: compile(ACCESSIBILITY_EVENT_RE)?,
        })
    }

    fn parse_middle(&self, middle: &str) -> (u32, u32, String) {
        let mut pair: Option<(u32, u32)> = None;
        // A uid prefix (`10123 1234-5678`) can fool the space variant, so prefer the
        // dashed form and always keep the LAST match (closest to the level letter).
        for caps in self.pair_dash_re.captures_iter(middle) {
            pair = Some((to_u32(&caps, "pid"), to_u32(&caps, "tid")));
        }
        if pair.is_none() {
            // `-v uid` with space-separated fields looks like `10123 1234 5678/com.foo`.
            // captures_iter() is non-overlapping and would lock onto the first pair, so
            // extract numeric tokens explicitly and take the LAST two (pid, tid).
            let nums: Vec<u32> = middle
                .split_whitespace()
                .filter_map(|token| {
                    self.num_re
                        .find(token)
                        .and_then(|m| m.as_str().parse::<u32>().ok())
                })
                .collect();
            if nums.len() >= 2 {
                pair = Some((nums[nums.len() - 2], nums[nums.len() - 1]));
            } else if let Some(caps) = self.pair_space_re.captures(middle) {
                pair = Some((to_u32(&caps, "pid"), to_u32(&caps, "tid")));
            }
        }
        let (mut pid, tid) = pair.unwrap_or((0, 0));
        if pid == 0 {
            if let Some(m) = self.num_re.find(middle) {
                pid = m.as_str().parse::<u32>().unwrap_or(0);
            }
        }
        let package = self
            .pkg_re
            .captures(middle)
            .and_then(|c| c.name("pkg").map(|m| m.as_str().to_string()))
            .unwrap_or_default();
        (pid, tid, package)
    }

    fn build_entry(&self, format: &Format, caps: &Captures<'_>, line: &str) -> LogEntry {
        let level = caps
            .name("level")
            .map(|m| m.as_str().to_uppercase())
            .unwrap_or_else(|| "?".to_string());
        let tag_raw = caps.name("tag").map(|m| m.as_str()).unwrap_or("").trim().to_string();
        let message = caps.name("msg").map(|m| m.as_str()).unwrap_or("").to_string();

        let date = caps
            .name("date")
            .map(|m| m.as_str().trim_end_matches(|c: char| c == ' ' || c == 'T'))
            .unwrap_or("");
        let time = caps
            .name("time")
            .map(|m| m.as_str().replace(',', "."))
            .unwrap_or_default();
        let timestamp = if date.is_empty() {
            time.clone()
        } else {
            format!("{} {}", date, time)
        };
        let time_key = normalize_time_value(&time);

        let middle = caps.name("middle").map(|m| m.as_str()).unwrap_or("");
        let (mut pid, mut tid, mut package) = self.parse_middle(middle);
        // Patterns that spell out pid/tid (plain threadtime, `-v process`) win over the
        // heuristic extraction from `middle`.
        if caps.name("pid").is_some() {
            pid = to_u32(caps, "pid");
        }
        if caps.name("tid").is_some() {
            tid = to_u32(caps, "tid");
        }
        if format.kind == FormatKind::Process {
            pid = to_u32(caps, "pid");
        }

        let mut tag = tag_raw.clone();
        if let Some(c) = self.tag_pid_re.captures(&tag_raw) {
            if pid == 0 {
                pid = c.get(1).and_then(|m| m.as_str().parse::<u32>().ok()).unwrap_or(0);
            }
            tag = self.tag_pid_re.replace_all(&tag_raw, "").trim().to_string();
        }

        // `brief` / `tag` / `time` output has no package column. For accessibility
        // foreground events, lift `<packageName>/<className>` out of the message into
        // structured fields so rules can filter on either side independently.
        let mut component = String::new();
        if let Some(event) = self.accessibility_re.captures(&message) {
            if package.is_empty() {
                package = event
                    .name("pkg")
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
            }
            component = event
                .name("component")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }

        LogEntry {
            index: 0,
            raw: line.to_string(),
            timestamp,
            time_key,
            level,
            tag,
            pid,
            tid,
            package,
            component,
            message,
            continuation: 0,
            parse_ok: true,
        }
    }

    fn parse_line(&self, line: &str) -> Option<LogEntry> {
        for format in &self.formats {
            if let Some(caps) = format.re.captures(line) {
                return Some(self.build_entry(format, &caps, line));
            }
        }
        None
    }

    fn looks_like_continuation(line: &str) -> bool {
        let t = line.trim_start();
        line.starts_with(' ')
            || line.starts_with('\t')
            || t.starts_with("at ")
            || t.starts_with("Caused by")
            || t.starts_with("Suppressed:")
            || t.starts_with("... ")
            || t.starts_with("java.")
            || t.starts_with("javax.")
            || t.starts_with("kotlin.")
            || t.starts_with("android.")
            || t.starts_with("---------")
    }

    /// Feed one physical line into the result. Public so unit tests and file streaming
    /// share exactly the same parsing behaviour.
    pub fn consume_line(&self, result: &mut ParseResult, line: &str) {
        result.total_lines += 1;

        if line.trim().is_empty() {
            if !result.entries.is_empty() {
                append_continuation(result, line);
            }
            return;
        }

        if let Some(caps) = self.divider_re.captures(line) {
            let label = caps.name("label").map(|m| m.as_str()).unwrap_or("");
            result.entries.push(LogEntry {
                index: 0,
                raw: line.to_string(),
                timestamp: String::new(),
                time_key: String::new(),
                level: "S".to_string(),
                tag: "logcat".to_string(),
                pid: 0,
                tid: 0,
                package: String::new(),
                component: String::new(),
                message: format!("beginning of {label}"),
                continuation: 0,
                parse_ok: true,
            });
            return;
        }

        if let Some(mut entry) = self.parse_line(line) {
            // Learn first, then fill: an `am_proc_start` row announces a process that will
            // print its own logs from the next line on, so the entry itself usually gains
            // nothing while everything after it does.
            result.processes.observe(&entry);
            result.processes.fill(&mut entry);
            result.entries.push(entry);
            return;
        }

        if Self::looks_like_continuation(line) && !result.entries.is_empty() {
            append_continuation(result, line);
            return;
        }

        // Preserve unknown standalone lines as first-class entries instead of dropping them.
        result.unparsed_lines += 1;
        result.entries.push(LogEntry {
            index: 0,
            raw: line.to_string(),
            timestamp: String::new(),
            time_key: String::new(),
            level: "?".to_string(),
            tag: String::new(),
            pid: 0,
            tid: 0,
            package: String::new(),
            component: String::new(),
            message: line.to_string(),
            continuation: 0,
            parse_ok: false,
        });
    }

    /// Line-iterator variant of the parser, used by unit tests. Live captures go through
    /// [`IncrementalParser`], which keeps the trailing entry back; files use `parse_file`
    /// so invalid UTF-8 can be handled byte by byte.
    #[cfg(test)]
    pub fn parse_lines<I, S>(&self, lines: I) -> ParseResult
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut result = ParseResult::default();
        for line in lines {
            self.consume_line(&mut result, line.as_ref());
        }
        finish(&mut result);
        result
    }

    /// Stream a file line by line. Invalid UTF-8 is replaced rather than aborting the read.
    ///
    /// Test-only shortcut for [`parse_file_with_progress`](Self::parse_file_with_progress):
    /// production imports always go through the progress-capable variant, and having both
    /// would leave a second code path nobody exercises.
    #[cfg(test)]
    pub fn parse_file(path: &Path) -> Result<ParseResult, String> {
        match Self::parse_file_with_progress(path, |_, _| true)? {
            ParseOutcome::Done(result) => Ok(result),
            // Unreachable with the always-continue callback above, but mapping it to an
            // error keeps the signature honest instead of `unreachable!()`-panicking.
            ParseOutcome::Cancelled => Err("导入已取消".to_string()),
        }
    }

    /// Same as [`parse_file`](Self::parse_file), but reports read progress and can be
    /// aborted.
    ///
    /// `on_progress(bytes_read, lines_parsed)` fires every `PROGRESS_STRIDE` lines — frequent
    /// enough to drive a smooth bar, rare enough that the callback (which crosses a thread
    /// boundary through a Tauri event) never dominates the parse. Returning `false` stops
    /// the read and yields [`ParseOutcome::Cancelled`]; nothing partial is returned, because a
    /// half-parsed log is not something the UI should be able to display.
    pub fn parse_file_with_progress(
        path: &Path,
        mut on_progress: impl FnMut(u64, u32) -> bool,
    ) -> Result<ParseOutcome, String> {
        let file = File::open(path).map_err(|e| format!("无法打开文件 {}: {e}", path.display()))?;
        let mut reader = BufReader::with_capacity(128 * 1024, file);
        let parser = LogcatParser::new()?;
        let mut result = ParseResult::default();
        let mut buf: Vec<u8> = Vec::new();
        let mut bytes_read: u64 = 0;
        let mut since_report: u32 = 0;

        loop {
            buf.clear();
            let read = reader
                .read_until(b'\n', &mut buf)
                .map_err(|e| format!("读取文件失败 {}: {e}", path.display()))?;
            if read == 0 {
                break;
            }
            bytes_read += read as u64;
            while let Some(&last) = buf.last() {
                if last == b'\n' || last == b'\r' {
                    buf.pop();
                } else {
                    break;
                }
            }
            let line = String::from_utf8_lossy(&buf).into_owned();
            parser.consume_line(&mut result, &line);

            since_report += 1;
            if since_report >= PROGRESS_STRIDE {
                since_report = 0;
                if !on_progress(bytes_read, result.total_lines) {
                    return Ok(ParseOutcome::Cancelled);
                }
            }
        }
        // Final report so the bar can reach 100% even for files below one stride.
        on_progress(bytes_read, result.total_lines);
        finish(&mut result);
        Ok(ParseOutcome::Done(result))
    }
}

/// Lines between two `parse_file_with_progress` callbacks. At ~2M lines/s this is a few
/// milliseconds apart, which is already smoother than the 100ms emit throttle above it.
const PROGRESS_STRIDE: u32 = 4096;

/// A file parse that the caller may abort part-way through.
pub enum ParseOutcome {
    Done(ParseResult),
    Cancelled,
}

fn append_continuation(result: &mut ParseResult, line: &str) {
    if let Some(last) = result.entries.last_mut() {
        last.raw.push('\n');
        last.raw.push_str(line);
        last.message.push('\n');
        last.message.push_str(line);
        last.continuation += 1;
    }
}

fn finish(result: &mut ParseResult) {
    for (i, entry) in result.entries.iter_mut().enumerate() {
        entry.index = i as u32;
    }
}

/// Incremental parser for streaming sources (`adb logcat`).
///
/// Continuation lines (stack frames, `Caused by:`, long JSON payloads) arrive *after* the
/// line that opened the entry, so the trailing entry is never final. Everything before it
/// is, which is what [`take_ready`](Self::take_ready) hands to the session. Without this
/// hold-back a live capture would split a stack trace away from its error line.
pub struct IncrementalParser {
    parser: LogcatParser,
    result: ParseResult,
}

impl IncrementalParser {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            parser: LogcatParser::new()?,
            result: ParseResult::default(),
        })
    }

    pub fn push_line(&mut self, line: &str) {
        self.parser.consume_line(&mut self.result, line);
    }

    /// Adopt a `ps` snapshot, so entries parsed from now on can resolve their package.
    /// Returns how many pid → process rows were stored.
    pub fn merge_process_snapshot(&mut self, text: &str) -> usize {
        self.result.processes.merge_snapshot(text)
    }

    pub fn total_lines(&self) -> u32 {
        self.result.total_lines
    }

    pub fn unparsed_lines(&self) -> u32 {
        self.result.unparsed_lines
    }

    /// Entries that can no longer change: all but the trailing one.
    pub fn take_ready(&mut self) -> Vec<LogEntry> {
        if self.result.entries.len() <= 1 {
            return Vec::new();
        }
        let tail = self.result.entries.pop();
        let ready = std::mem::take(&mut self.result.entries);
        if let Some(tail) = tail {
            self.result.entries.push(tail);
        }
        ready
    }

    /// Close the stream and return the trailing entry together with the line counters.
    /// `index` is assigned by the caller, which knows the session offset.
    pub fn finish(mut self) -> (Vec<LogEntry>, u32, u32) {
        let entries = std::mem::take(&mut self.result.entries);
        (entries, self.result.total_lines, self.result.unparsed_lines)
    }
}

/// Distinct values for the filter dropdowns, capped so a huge file cannot flood the UI.
pub fn collect_facets(entries: &[LogEntry]) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut levels = BTreeSet::new();
    let mut tags = BTreeSet::new();
    let mut pids = BTreeSet::new();
    let mut packages = BTreeSet::new();
    // No special case needed here: `build_entry` already lifts the accessibility
    // `packageName` into `LogEntry.package`, so those apps appear as ordinary facets.
    for e in entries {
        levels.insert(e.level.clone());
        if !e.tag.is_empty() && tags.len() < 400 {
            tags.insert(e.tag.clone());
        }
        if e.pid > 0 && pids.len() < 400 {
            pids.insert(e.pid.to_string());
        }
        if !e.package.is_empty() && packages.len() < 400 {
            packages.insert(e.package.clone());
        }
    }
    (
        levels.into_iter().collect(),
        tags.into_iter().collect(),
        pids.into_iter().collect(),
        packages.into_iter().collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lines: &[&str]) -> ParseResult {
        LogcatParser::new().unwrap().parse_lines(lines.to_vec())
    }

    #[test]
    fn parses_threadtime_with_package() {
        let r = parse(&["11-12 10:23:45.123  1234-5678/com.example.app D/MyTag: hello world"]);
        assert_eq!(r.entries.len(), 1);
        let e = &r.entries[0];
        assert_eq!(e.level, "D");
        assert_eq!(e.tag, "MyTag");
        assert_eq!(e.pid, 1234);
        assert_eq!(e.tid, 5678);
        assert_eq!(e.package, "com.example.app");
        assert_eq!(e.message, "hello world");
        assert_eq!(e.time_key, "10:23:45.123");
    }

    #[test]
    fn parses_time_format_pid_in_tag() {
        let r = parse(&["11-12 10:23:45.123 E/AndroidRuntime( 3210): FATAL EXCEPTION"]);
        let e = &r.entries[0];
        assert_eq!(e.level, "E");
        assert_eq!(e.tag, "AndroidRuntime");
        assert_eq!(e.pid, 3210);
    }

    #[test]
    fn parses_brief_and_year_and_uid() {
        let r = parse(&[
            "I/BriefTag(  99): ok",
            "2026-11-12 10:24:01.500 10123 1234-5678/com.foo W/WarnTag: watch out",
        ]);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].tag, "BriefTag");
        assert_eq!(r.entries[1].level, "W");
        assert_eq!(r.entries[1].pid, 1234);
        assert_eq!(r.entries[1].tid, 5678);
        assert_eq!(r.entries[1].package, "com.foo");
    }

    #[test]
    fn parses_launcher_top_app_changed_event() {
        // Real ZUI Launcher foreground-app event, emitted with `adb logcat -v brief`.
        let line = "11-03 18:02:29.792 I/AccessibilityData( 2108): onTopAppChanged: com.example.launcher/com.example.quickstep.RecentsActivity";
        let r = parse(&[line]);
        assert_eq!(r.entries.len(), 1);
        let e = &r.entries[0];
        assert_eq!(e.level, "I");
        assert_eq!(e.tag, "AccessibilityData");
        assert_eq!(e.pid, 2108);
        assert_eq!(e.time_key, "18:02:29.792");
        assert!(e.message.contains("onTopAppChanged: com.example.launcher/"));
        assert_eq!(e.package, "com.example.launcher");
        assert_eq!(e.component, "com.example.quickstep.RecentsActivity");
        let (_, _, _, packages) = collect_facets(&r.entries);
        assert!(packages.contains(&"com.example.launcher".to_string()));
    }

    #[test]
    fn accessibility_events_split_package_and_component_class() {
        // Any observed app, and className is not necessarily an Activity (FrameLayout
        // here is a window / popup component reported by AccessibilityEvent).
        let r = parse(&[
            "11-03 18:02:29.830 I/AccessibilityData( 2108): onTopAppChanged: com.example.sample.launch/com.example.sample.launch.ui.main.MainActivity",
            "11-03 18:25:05.005 I/AccessibilityData( 2108): onTopAppChanged: com.example.sample.web/android.widget.FrameLayout",
        ]);
        assert_eq!(r.entries.len(), 2);
        let first = &r.entries[0];
        assert_eq!(first.package, "com.example.sample.launch");
        assert_eq!(first.component, "com.example.sample.launch.ui.main.MainActivity");
        assert_eq!(first.tag, "AccessibilityData");
        let second = &r.entries[1];
        assert_eq!(second.package, "com.example.sample.web");
        assert_eq!(second.component, "android.widget.FrameLayout");
        let (_, _, _, packages) = collect_facets(&r.entries);
        assert_eq!(packages.len(), 2, "both observed apps must be facets: {packages:?}");
    }

    #[test]
    fn merges_stack_trace_into_parent_entry() {
        let r = parse(&[
            "11-12 10:25:00.001  1234-5678/com.foo E/AndroidRuntime: FATAL EXCEPTION: main",
            "    at com.foo.MainActivity.onCreate(MainActivity.kt:42)",
            "Caused by: java.lang.NullPointerException",
            "    ... 12 more",
        ]);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].continuation, 3);
        assert!(r.entries[0].message.contains("MainActivity.kt:42"));
        assert!(r.entries[0].raw.contains("... 12 more"));
    }

    #[test]
    fn divider_is_a_separate_entry() {
        let r = parse(&[
            "--------- beginning of crash",
            "11-12 10:26:00.000  1-2/com.foo D/T: after divider",
        ]);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].level, "S");
        assert_eq!(r.entries[1].message, "after divider");
    }

    #[test]
    fn long_header_and_unknown_line_are_preserved() {
        let r = parse(&[
            "[ 11-12 10:27:00.123  1234:5678 D/LongTag ]",
            "    long format body",
            "this line is not logcat at all",
        ]);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].tag, "LongTag");
        assert_eq!(r.entries[0].continuation, 1);
        assert_eq!(r.entries[1].level, "?");
        assert_eq!(r.unparsed_lines, 1);
    }

    #[test]
    fn parses_plain_threadtime_without_package_column() {
        // What `adb logcat -v threadtime` actually prints on a device: no `/`, no package.
        let e = parse(&[
            "11-12 17:28:02.013  1080  1161 D PowerGuruService: Already added to Heartbeat Alarms",
            "11-12 17:28:02.014  1080  1161 D PowerGuruService: alarm info: packageName = com.example.sample.launch",
        ])
        .entries;
        assert_eq!(e[0].level, "D");
        assert_eq!(e[0].tag, "PowerGuruService");
        assert_eq!(e[0].pid, 1080);
        assert_eq!(e[0].tid, 1161);
        assert_eq!(e[0].timestamp, "11-12 17:28:02.013");
        assert_eq!(e[0].time_key, "17:28:02.013");
        assert!(e[0].message.starts_with("Already added"));
        // The colon inside the message must not be mistaken for the tag separator.
        assert_eq!(e[1].tag, "PowerGuruService");
        assert!(e[1].message.contains("packageName = com.example"));
    }

    #[test]
    fn plain_threadtime_wins_over_a_slash_looking_message() {
        // Without the plain pattern being tried first, the lazy `middle` of the slash
        // pattern would swallow the real tag and report `http` as the tag.
        let r = parse(&["11-12 17:28:02.013  1080  1161 E OkHttp: GET /api failed E/http: 500"]);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].tag, "OkHttp");
        assert_eq!(r.entries[0].pid, 1080);
        assert!(r.entries[0].message.contains("E/http: 500"));
    }

    #[test]
    fn normalises_time_bounds() {
        assert_eq!(normalize_time_value("10:23:45.12"), "10:23:45.120");
        assert_eq!(normalize_time_value("10:23"), "10:23:00.000");
        assert_eq!(normalize_time_value("11-12 10:23:45,123"), "10:23:45.123");
    }

    #[test]
    fn incremental_parser_holds_back_the_trailing_entry() {
        let mut parser = IncrementalParser::new().unwrap();
        parser.push_line("11-12 10:25:00.001  1234-5678/com.foo E/AndroidRuntime: FATAL EXCEPTION: main");
        // The trailing entry may still receive stack frames, so nothing is ready yet.
        assert!(parser.take_ready().is_empty());

        parser.push_line("    at com.foo.MainActivity.onCreate(MainActivity.kt:42)");
        // Still the same entry — the continuation must land on it, not on the caller.
        assert!(parser.take_ready().is_empty());

        parser.push_line("11-12 10:25:00.500  1234-5678/com.foo I/OkHttp: next event");
        let ready = parser.take_ready();
        assert_eq!(ready.len(), 1, "only the finished entry is released");
        assert_eq!(ready[0].continuation, 1, "stack frame stays with its error line");
        assert!(ready[0].message.contains("MainActivity.kt:42"));

        let (tail, total, unparsed) = parser.finish();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].message, "next event");
        assert_eq!(total, 3);
        assert_eq!(unparsed, 0);
    }

    /* ---------------------------------------------------------- pid → package ---- */

    /// `adb shell ps -A -o PID,NAME` on a real device, trimmed to the interesting rows.
    const PS_SNAPSHOT: &str = "\
  PID NAME
    1 init
   12 [kworker/0:1]
  520 statsd
  815 vendor.lenovo.hardware.battery@1.0-service
 3924 com.android.remoteprovisioner
 5000 com.tencent.mm:push
 6000 com.example.sample.student
";

    #[test]
    fn ps_snapshot_maps_app_pids_and_skips_kernel_threads() {
        let mut index = ProcessIndex::default();
        let adopted = index.merge_snapshot(PS_SNAPSHOT);
        // 6 data rows: the header, `init` and the bracketed kernel thread contribute
        // nothing; every other row is stored (filtering happens when a name is used).
        assert_eq!(adopted, 6, "header / kernel thread must not be adopted");
        assert_eq!(index.by_pid.get(&12), None, "kernel threads are not processes to name");

        let mut entry = LogEntry { pid: 6000, ..Default::default() };
        index.fill(&mut entry);
        assert_eq!(entry.package, "com.example.sample.student");

        // A sub-process is the same application.
        let mut push = LogEntry { pid: 5000, ..Default::default() };
        index.fill(&mut push);
        assert_eq!(push.package, "com.tencent.mm", "`:push` collapses to its package");

        // System processes and vendor HAL services have no package, so they stay unnamed
        // instead of polluting the package filter.
        for pid in [1u32, 520, 815] {
            let mut system = LogEntry { pid, ..Default::default() };
            index.fill(&mut system);
            assert!(system.package.is_empty(), "pid {pid} must not become a package");
        }
    }

    #[test]
    fn am_proc_start_names_every_following_line() {
        // Real events-buffer rows: `[userId, pid, uid, processName, type, component]`.
        let r = parse(&[
            "11-12 17:57:17.386 19484 19519 I am_proc_start: [0,13800,10174,com.example.sample.student,next-top-activity,{com.example.sample.student/com.example.sample.student.MainActivity}]",
            "11-12 17:57:18.001 13800 13800 D OkHttp: GET /api/paper",
            "11-12 17:57:19.002 13800 13999 D OkHttp: response 200",
        ]);
        // The announcement itself is written by system_server, so it has no package...
        assert_eq!(r.entries[0].package, "");
        // ...but every line from the announced pid resolves.
        assert_eq!(r.entries[1].package, "com.example.sample.student");
        assert_eq!(r.entries[2].package, "com.example.sample.student");
    }

    #[test]
    fn activity_manager_text_rows_name_processes_and_forget_dead_ones() {
        let r = parse(&[
            "11-12 17:57:17.386 19484 19519 I ActivityManager: Start proc 13800:com.example.sample.student/u0a174 for next-top-activity {com.example.sample.student/com.example.sample.student.MainActivity}",
            "11-12 17:57:18.000 13800 13800 D OkHttp: first life",
            // Killed, then a *new* app reuses the same pid.
            "11-12 17:58:00.000 19484 19518 I ActivityManager: Killing 13800:com.example.sample.student/u0a174 (adj 200): stop com.example.sample.student due to installPackageLI",
            "11-12 17:58:01.000 19484 19519 I ActivityManager: Start proc 13800:com.other.app/u0a200 for broadcast {com.other.app/.BootReceiver}",
            "11-12 17:58:02.000 13800 13800 D OkHttp: second life",
        ]);
        assert_eq!(r.entries[1].package, "com.example.sample.student");
        assert_eq!(
            r.entries[4].package, "com.other.app",
            "a recycled pid must not inherit the dead app's name"
        );
    }

    #[test]
    fn a_package_spelled_out_in_the_line_wins_over_the_index() {
        let mut index = ProcessIndex::default();
        index.merge_snapshot(" 2108 com.android.systemui\n");

        // Slash format: the package is authoritative, the index must not overwrite it.
        let mut entry = LogEntry {
            pid: 2108,
            package: "com.example.launcher".into(),
            ..Default::default()
        };
        index.fill(&mut entry);
        assert_eq!(entry.package, "com.example.launcher");

        // Same for an accessibility foreground event, whose package names the app on
        // screen rather than the process that printed the line.
        let r = parse(&["11-03 18:02:29.792 I/AccessibilityData( 2108): onTopAppChanged: com.example.launcher/com.example.quickstep.RecentsActivity"]);
        assert_eq!(r.entries[0].package, "com.example.launcher");
        assert_eq!(r.entries[0].component, "com.example.quickstep.RecentsActivity");
    }

    #[test]
    fn file_sessions_learn_the_mapping_too() {
        // No `ps` available for a file, so everything comes from the log itself.
        let r = parse(&[
            "11-12 10:00:00.000 19484 19519 I am_proc_start: [0,7001,10174,com.example.app,service,{com.example.app/.SyncService}]",
            "11-12 10:00:01.000 7001 7001 E AndroidRuntime: FATAL EXCEPTION: main",
        ]);
        let (_, _, _, packages) = collect_facets(&r.entries);
        assert!(packages.contains(&"com.example.app".to_string()), "facets: {packages:?}");
    }

    /// Builds a file big enough to cross `PROGRESS_STRIDE`, since a callback-driven progress
    /// bar that never fires for real-sized logs would be useless.
    fn write_synthetic_log(tag: &str, lines: usize) -> std::path::PathBuf {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!(
            "loglens-progress-{}-{tag}.log",
            std::process::id()
        ));
        let mut file = std::fs::File::create(&path).expect("应能创建临时文件");
        for i in 0..lines {
            writeln!(
                file,
                "11-12 10:00:{:02}.{:03}  1000  1000 D/Progress: line {}",
                i % 60,
                i % 1000,
                i
            )
            .expect("应能写入临时文件");
        }
        path
    }

    #[test]
    fn parse_reports_progress_and_finishes_at_the_file_size() {
        let lines = (PROGRESS_STRIDE as usize) * 2 + 10;
        let path = write_synthetic_log("forward", lines);
        let total_bytes = std::fs::metadata(&path).expect("能读取元数据").len();

        let mut reports: Vec<(u64, u32)> = Vec::new();
        let outcome = LogcatParser::parse_file_with_progress(&path, |bytes, entries| {
            reports.push((bytes, entries));
            true
        })
        .expect("解析应成功");
        let _ = std::fs::remove_file(&path);

        let ParseOutcome::Done(parsed) = outcome else {
            panic!("未取消的解析不应返回 Cancelled");
        };
        assert_eq!(parsed.entries.len(), lines);

        // The stride callback plus the final one: with 2*stride+10 lines that is 3 reports.
        assert!(reports.len() >= 3, "进度回调次数过少: {reports:?}");
        assert!(
            reports.windows(2).all(|w| w[0].0 <= w[1].0),
            "已读字节必须单调递增: {reports:?}"
        );
        // The last report is what the bar uses to reach 100%; it must be the real file size.
        assert_eq!(reports.last().map(|r| r.0), Some(total_bytes));
        assert_eq!(
            reports.last().map(|r| r.1),
            Some(parsed.total_lines),
            "最终行数应与解析结果一致"
        );
    }

    #[test]
    fn cancelling_stops_the_parse_and_returns_nothing() {
        let path = write_synthetic_log("cancel", (PROGRESS_STRIDE as usize) * 3);
        let mut calls = 0;
        let outcome = LogcatParser::parse_file_with_progress(&path, |_, _| {
            calls += 1;
            // Cancel on the first report, i.e. before the file has been read through.
            false
        })
        .expect("取消不是错误");
        let _ = std::fs::remove_file(&path);

        assert_eq!(calls, 1, "取消后应立即停止上报，不再继续读");
        assert!(matches!(outcome, ParseOutcome::Cancelled));
    }
}
