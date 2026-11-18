//! Client self-diagnostics: one bounded local file for every failure and every
//! **flow checkpoint** in this app.
//!
//! This is deliberately not a general logging framework. It records four things the
//! user can no longer see once the window is gone:
//!   1. Rust panics (global hook, with backtrace),
//!   2. uncaught frontend errors forwarded over IPC,
//!   3. Tauri commands that returned `Err`,
//!   4. `info`-level flow checkpoints (`record_flow`) at the boundaries of every
//!      user-visible workflow — startup, file/session, device streams, capture, rules,
//!      import/export, proxy restore and UI notices. They name the stage reached and
//!      carry counters/status, so "看不到内容 / 点了没反应" can be traced without a
//!      debugger. Flow records hold counters only — never URLs, headers or bodies.
//!
//! Records are kept in memory and mirrored as JSONL at
//! `<app_config_dir>/error-log.jsonl` (same directory as `rules.json`), so a support
//! bundle can be assembled without asking the user to dig through OS-specific paths.
//!
//! Bounded on purpose: `MAX_RECORDS` caps the file; identical records inside a short
//! window fold into a `count` (flow checkpoints keep their newest counters). This makes
//! high-frequency boundaries safe without introducing a logging framework.
//! See `.trellis/spec/backend/logging-guidelines.md` for what must NOT end up here.

use crate::model::ErrorRecord;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const LOG_FILE: &str = "error-log.jsonl";

/// Hard cap on persisted records. Rewriting the file on every record is fine at this size.
const MAX_RECORDS: usize = 500;

/// Failures with the same signature inside this window fold into one record (`count += 1`).
const FOLD_WINDOW_MS: i64 = 10_000;

/// How many trailing records to inspect when folding. Keeps a 500-line scan out of the
/// panic path while still collapsing the alternating case (poll error, other error, ...).
const FOLD_SCAN: usize = 64;

struct State {
    path: PathBuf,
    records: Vec<ErrorRecord>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Poisoned lock must never turn a diagnostics call into a second panic — especially
/// from inside a panic hook. Recover the inner guard like the shutdown paths do.
fn lock() -> Option<std::sync::MutexGuard<'static, State>> {
    STATE.get().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()))
}

fn persist(state: &State) {
    let mut text = String::with_capacity(state.records.len() * 160);
    for record in &state.records {
        match serde_json::to_string(record) {
            Ok(line) => {
                text.push_str(&line);
                text.push('\n');
            }
            Err(e) => eprintln!("[error-log] 序列化记录失败: {e}"),
        }
    }
    // Write whole file rather than appending: folding updates an existing record's
    // count, and trimming drops the oldest ones. 500 lines makes this cheap.
    if let Err(e) = fs::write(&state.path, text) {
        eprintln!("[error-log] 写入失败 {}: {e}", state.path.display());
    }
}

/// Install the hook and load any records from a previous run. Called once from `.setup()`.
/// Failures to initialise are reported to stderr — error logging must never abort startup.
pub fn init(app: &AppHandle) {
    let path = match app.path().app_config_dir() {
        Ok(dir) => dir.join(LOG_FILE),
        Err(e) => {
            eprintln!("[error-log] 无法定位配置目录: {e}");
            return;
        }
    };
    if let Err(e) = fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new("."))) {
        eprintln!("[error-log] 无法创建日志目录: {e}");
        return;
    }

    let mut records: Vec<ErrorRecord> = Vec::new();
    match fs::read_to_string(&path) {
        Ok(text) => {
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<ErrorRecord>(line) {
                    Ok(record) => records.push(record),
                    Err(_) => eprintln!("[error-log] 跳过损坏的历史记录"),
                }
            }
        }
        // First launch is the normal case.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[error-log] 读取历史记录失败: {e}"),
    }
    if records.len() > MAX_RECORDS {
        let drain = records.len() - MAX_RECORDS;
        records.drain(..drain);
    }

    if STATE
        .set(Mutex::new(State { path: path.clone(), records }))
        .is_err()
    {
        // Already initialised (only reachable in tests / embedded reuse).
        return;
    }

    install_panic_hook();
    persist(&lock().expect("STATE just set"));
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = panic_payload(info);
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let mut detail = String::new();
        if !location.is_empty() {
            detail.push_str("位置: ");
            detail.push_str(&location);
        }
        if !backtrace.is_empty() {
            if !detail.is_empty() {
                detail.push('\n');
            }
            detail.push_str("调用栈:\n");
            detail.push_str(&backtrace);
        }
        record("panic", "rust", "panic", &payload, &detail);
        // Keep the normal stderr output for developers running `tauri dev`.
        default_hook(info);
    }));
}

fn panic_payload(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "未知 panic".to_string()
    }
}

/// Append one record to memory + disk. `level`: panic | error | warning.
/// `source`: rust | frontend. `scope`: command name, module, or JS handler name.
pub fn record(level: &str, source: &str, scope: &str, message: &str, detail: &str) {
    let Some(mut state) = lock() else {
        // Not initialised yet (pre-`.setup()` panic): stderr is the only channel.
        eprintln!("[error-log:{level}/{source}:{scope}] {message}");
        return;
    };
    let ts = now_millis();

    // Fold a repeat of the same failure instead of emitting another line. This is what
    // keeps the 800ms poll commands from swamping the log during teardown.
    let from = state.records.len().saturating_sub(FOLD_SCAN);
    if let Some(existing) = state.records[from..].iter_mut().rev().find(|r| {
        r.level == level
            && r.source == source
            && r.scope == scope
            && r.message == message
            && (ts - r.ts).abs() <= FOLD_WINDOW_MS
    }) {
        existing.count += 1;
        existing.ts = ts;
        // A flow checkpoint is the newest snapshot of a step, not a repeated failure:
        // keep its latest counters so "the poll right before it went blank" survives.
        if level == "info" {
            existing.detail = detail.to_string();
        }
        persist(&state);
        return;
    }

    let id = format!("e{}-{}", ts, NEXT_ID.fetch_add(1, Ordering::Relaxed));
    state.records.push(ErrorRecord {
        id,
        ts,
        level: level.to_string(),
        source: source.to_string(),
        scope: scope.to_string(),
        message: message.to_string(),
        detail: detail.to_string(),
        count: 1,
    });
    if state.records.len() > MAX_RECORDS {
        let drain = state.records.len() - MAX_RECORDS;
        state.records.drain(..drain);
    }
    persist(&state);
}

/// Append one flow checkpoint. `scope` is a dotted step name (`<module>.<step>`),
/// `message` names the state, and `detail` carries only counts / booleans / ids —
/// never URLs, headers or bodies. All workflows share `source=flow`; the UI filters
/// them with level `info` or source `flow`.
pub fn record_flow(scope: &str, message: &str, detail: &str) {
    record("info", "flow", scope, message, detail);
}

/// Flow checkpoint that also stays visible in a foreground `tauri dev` terminal.
/// Use this when replacing an `eprintln!` that describes a workflow boundary; the
/// terminal copy keeps its prefixed, greppable wording.
pub fn flow_echo(scope: &str, message: &str, detail: &str) {
    eprintln!("[{}] {}{}{}", scope, message, if detail.is_empty() { "" } else { " " }, detail);
    record_flow(scope, message, detail);
}

/// Newest first, so the UI can render straight down the list.
pub fn list() -> Vec<ErrorRecord> {
    let Some(state) = lock() else {
        return Vec::new();
    };
    let mut records = state.records.clone();
    records.reverse();
    records
}

pub fn clear() -> Result<(), String> {
    let mut state = lock().ok_or_else(|| "错误日志尚未初始化".to_string())?;
    state.records.clear();
    // Truncate in place rather than delete: keeps the file inode/path stable for
    // anything the user may already have open.
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&state.path)
        .map_err(|e| format!("清空错误日志失败: {e}"))?;
    Ok(())
}

pub fn location() -> Result<String, String> {
    let state = lock().ok_or_else(|| "错误日志尚未初始化".to_string())?;
    Ok(state.path.to_string_lossy().to_string())
}

/// Human-readable export — intentionally not JSONL, so it can be pasted into a ticket.
pub fn export_text() -> String {
    let records = list();
    let mut out = String::new();
    out.push_str("LogLens 诊断日志\n");
    out.push_str(&format!("导出时间: {}\n", now_millis()));
    out.push_str(&format!("记录数: {}\n", records.len()));
    out.push_str("------------------------------------------------------------\n");
    for r in records {
        out.push_str(&format!(
            "[{}] [{}] [{}] {} ",
            r.ts, r.level, r.source, r.scope
        ));
        if r.count > 1 {
            out.push_str(&format!("(x{}) ", r.count));
        }
        out.push('\n');
        out.push_str(&r.message);
        out.push('\n');
        if !r.detail.is_empty() {
            out.push_str(&r.detail);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

    /// Tests exercise a private State directly — the global STATE is a process singleton
    /// installed with a real AppHandle and must not be touched by unit tests.
    fn temp_state() -> (State, PathBuf) {
        let n = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "loglens-error-log-test-{}-{}.jsonl",
            std::process::id(),
            n
        ));
        let _ = fs::remove_file(&path);
        (State { path: path.clone(), records: Vec::new() }, path)
    }

    #[test]
    fn record_folds_repeats_and_bumps_count() {
        let (mut state, _path) = temp_state();
        // Mirror `record`'s fold logic on a private state (no global singleton).
        let ts = now_millis();
        state.records.push(ErrorRecord {
            id: "e1".into(),
            ts,
            level: "error".into(),
            source: "rust".into(),
            scope: "query_logs".into(),
            message: "状态锁异常".into(),
            detail: String::new(),
            count: 1,
        });
        let from = state.records.len().saturating_sub(FOLD_SCAN);
        let found = state.records[from..].iter_mut().rev().find(|r| {
            r.level == "error"
                && r.source == "rust"
                && r.scope == "query_logs"
                && r.message == "状态锁异常"
                && (ts - r.ts).abs() <= FOLD_WINDOW_MS
        });
        assert!(found.is_some(), "同一签名在窗口内应被折叠");
        found.unwrap().count += 1;
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].count, 2);
    }

    #[test]
    fn flow_folding_keeps_the_latest_snapshot() {
        let (mut state, _path) = temp_state();
        let ts = now_millis();
        state.records.push(ErrorRecord {
            id: "f1".into(),
            ts,
            level: "info".into(),
            source: "flow".into(),
            scope: "capture.poll.net-1".into(),
            message: "采集轮询".into(),
            detail: "rows=0".into(),
            count: 1,
        });
        let from = state.records.len().saturating_sub(FOLD_SCAN);
        let found = state.records[from..].iter_mut().rev().find(|r| {
            r.level == "info"
                && r.source == "flow"
                && r.scope == "capture.poll.net-1"
                && r.message == "采集轮询"
                && (ts - r.ts).abs() <= FOLD_WINDOW_MS
        });
        let found = found.expect("同一采集步骤在窗口内应折叠");
        found.count += 1;
        found.detail = "rows=7".into();
        assert_eq!(state.records.len(), 1, "轮询不能每 800ms 新增一条");
        assert_eq!(state.records[0].detail, "rows=7", "折叠后应保留最新计数");
    }

    #[test]
    fn jsonl_round_trip_keeps_all_fields() {
        let (_state, path) = temp_state();
        let record = ErrorRecord {
            id: "e9".into(),
            ts: 1_700_000_000_000,
            level: "panic".into(),
            source: "rust".into(),
            scope: "panic".into(),
            message: "assertion failed".into(),
            detail: "调用栈:\n  at main".into(),
            count: 3,
        };
        let with_one = State { path: path.clone(), records: vec![record.clone()] };
        persist(&with_one);
        let text = fs::read_to_string(&path).expect("JSONL should be written");
        assert_eq!(text.lines().count(), 1);
        let back: ErrorRecord = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(back.id, record.id);
        assert_eq!(back.count, 3);
        assert_eq!(back.detail, record.detail);
    }

    #[test]
    fn export_text_includes_scope_and_count() {
        let records = vec![ErrorRecord {
            id: "e2".into(),
            ts: 1_700_000_000_000,
            level: "error".into(),
            source: "frontend".into(),
            scope: "window.onerror".into(),
            message: "boom".into(),
            detail: String::new(),
            count: 5,
        }];
        let mut out = String::from("LogLens 错误日志\n");
        for r in &records {
            out.push_str(&format!("[{}] [{}] [{}] {} (x{})\n{}\n", r.ts, r.level, r.source, r.scope, r.count, r.message));
        }
        assert!(out.contains("window.onerror"));
        assert!(out.contains("(x5)"));
        assert!(out.contains("boom"));
    }

    #[test]
    fn max_records_is_bounded() {
        assert!(MAX_RECORDS <= 1000, "错误日志必须有硬上限");
        assert!(FOLD_WINDOW_MS > 0);
    }
}
