//! Android Logcat parser.
//!
//! Supported `adb logcat -v` formats:
//!   threadtime (with/without year, with/without package, with/without uid)
//!   time, brief, tag, process, long
//!
//! Design decisions:
//! - Parsing is deliberately tolerant: an unrecognised line is never dropped.
//! - Indented lines (stack frames, `Caused by`, JSON payloads, ...) are merged into the
//!   previous entry so a filter hit keeps its stack trace context.
//! - `--------- beginning of ...` becomes a synthetic `S/logcat` entry instead of
//!   polluting the previous message.

use crate::model::LogEntry;
use regex::{Captures, Regex};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Default)]
pub struct ParseResult {
    pub entries: Vec<LogEntry>,
    /// Physical lines read from the file, including continuation lines.
    pub total_lines: u32,
    /// Standalone lines that could not be parsed into a known format.
    pub unparsed_lines: u32,
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
        let (mut pid, tid, mut package) = self.parse_middle(middle);
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

        if let Some(entry) = self.parse_line(line) {
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

    /// Line-iterator variant of the parser. Only the unit tests use it today; the app
    /// reads files through `parse_file` so it can handle invalid UTF-8 byte by byte.
    /// Keep it test-only until another log source (e.g. a pasted buffer or adb stream)
    /// needs a line-based entry point.
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
    pub fn parse_file(path: &Path) -> Result<ParseResult, String> {
        let file = File::open(path).map_err(|e| format!("无法打开文件 {}: {e}", path.display()))?;
        let mut reader = BufReader::with_capacity(128 * 1024, file);
        let parser = LogcatParser::new()?;
        let mut result = ParseResult::default();
        let mut buf: Vec<u8> = Vec::new();

        loop {
            buf.clear();
            let read = reader
                .read_until(b'\n', &mut buf)
                .map_err(|e| format!("读取文件失败 {}: {e}", path.display()))?;
            if read == 0 {
                break;
            }
            while let Some(&last) = buf.last() {
                if last == b'\n' || last == b'\r' {
                    buf.pop();
                } else {
                    break;
                }
            }
            let line = String::from_utf8_lossy(&buf).into_owned();
            parser.consume_line(&mut result, &line);
        }
        finish(&mut result);
        Ok(result)
    }
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
        let line = "11-03 18:02:29.792 I/AccessibilityData( 2108): onTopAppChanged: com.zui.launcher/com.zui.quickstep.RecentsActivity";
        let r = parse(&[line]);
        assert_eq!(r.entries.len(), 1);
        let e = &r.entries[0];
        assert_eq!(e.level, "I");
        assert_eq!(e.tag, "AccessibilityData");
        assert_eq!(e.pid, 2108);
        assert_eq!(e.time_key, "18:02:29.792");
        assert!(e.message.contains("onTopAppChanged: com.zui.launcher/"));
        assert_eq!(e.package, "com.zui.launcher");
        assert_eq!(e.component, "com.zui.quickstep.RecentsActivity");
        let (_, _, _, packages) = collect_facets(&r.entries);
        assert!(packages.contains(&"com.zui.launcher".to_string()));
    }

    #[test]
    fn accessibility_events_split_package_and_component_class() {
        // Any observed app, and className is not necessarily an Activity (FrameLayout
        // here is a window / popup component reported by AccessibilityEvent).
        let r = parse(&[
            "11-03 18:02:29.830 I/AccessibilityData( 2108): onTopAppChanged: com.hljzjx.edu.student.launch/com.hljzjx.launch.ui.main.MainActivity",
            "11-03 18:25:05.005 I/AccessibilityData( 2108): onTopAppChanged: com.hailiang.xxb.web.app/android.widget.FrameLayout",
        ]);
        assert_eq!(r.entries.len(), 2);
        let first = &r.entries[0];
        assert_eq!(first.package, "com.hljzjx.edu.student.launch");
        assert_eq!(first.component, "com.hljzjx.launch.ui.main.MainActivity");
        assert_eq!(first.tag, "AccessibilityData");
        let second = &r.entries[1];
        assert_eq!(second.package, "com.hailiang.xxb.web.app");
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
    fn normalises_time_bounds() {
        assert_eq!(normalize_time_value("10:23:45.12"), "10:23:45.120");
        assert_eq!(normalize_time_value("10:23"), "10:23:00.000");
        assert_eq!(normalize_time_value("11-12 10:23:45,123"), "10:23:45.123");
    }
}
