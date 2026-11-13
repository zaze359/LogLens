//! Live `adb logcat` capture.
//!
//! `adb logcat` is spawned as a long-running child; a reader thread parses its stdout
//! line by line and parks finished entries in a shared buffer. The IPC side drains that
//! buffer on a timer, so the UI can append new lines without re-reading the whole session.
//!
//! Why a buffer instead of writing straight into the log session: the reader thread must
//! not hold the session mutex while blocked on a pipe, and the session lives inside Tauri
//! managed state. The thread owns only the parser and an `Arc<Mutex<StreamBuffer>>`.

use crate::adb;
use crate::logcat::IncrementalParser;
use crate::model::{LogEntry, StreamStatus};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Hand entries to the polling side when either threshold trips.
///
/// Batch size alone is wrong for a log tail: it is latency-bound, not throughput-bound.
/// A quiet phone emits ~14 lines/s, so "every 128 lines" froze the UI for ~9 seconds at a
/// time (and `total_lines` with it, since the counters ride along with the flush). A
/// 120ms time trigger keeps the UI within one poll interval of the device while a burst
/// still coalesces into large batches. The mutex is uncontended — the polling side holds
/// it for microseconds once per tick — so the extra lock traffic costs nothing measurable.
const FLUSH_EVERY_LINES: u32 = 16;
const FLUSH_EVERY: Duration = Duration::from_millis(120);

/// How often the pid → package map is refreshed while a capture runs.
///
/// The snapshot costs ~120ms and returns ~480 rows, so it is cheap, but it must happen more
/// than once: apps start and die while the user reproduces a bug, and each new pid needs a
/// name. Refreshes are only attempted when a log line arrives, i.e. when something is
/// actually happening — an idle phone issues no `ps` calls at all.
const PS_REFRESH: Duration = Duration::from_secs(3);

/// Entries parsed by the reader thread but not yet moved into the session.
#[derive(Default)]
pub struct StreamBuffer {
    pending: Vec<LogEntry>,
    total_lines: u32,
    unparsed_lines: u32,
    /// Set by the reader thread when stdout reaches EOF (either adb exited or was killed).
    finished: bool,
    /// Non-zero exit code, or an I/O error while reading.
    error: String,
    stderr: String,
}

pub struct DeviceStream {
    child: Child,
    shared: Arc<Mutex<StreamBuffer>>,
    session_id: String,
    /// Device this stream is attached to. A device may only be captured once at a time, so
    /// the command layer uses this to refuse a second capture of the same serial.
    serial: String,
}

impl DeviceStream {
    pub fn serial(&self) -> &str {
        &self.serial
    }

    pub fn spawn(
        adb_path: &Path,
        args: Vec<String>,
        serial: &str,
        session_id: String,
    ) -> Result<Self, String> {
        // Taken before the child starts, so the ring buffer `adb logcat` replays on startup
        // resolves its packages too — otherwise the first screenful would be package-less.
        let snapshot = adb::process_snapshot(adb_path, serial);

        let mut cmd = adb::command(adb_path);
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("无法启动 adb logcat: {e}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法读取 adb logcat 输出".to_string())?;
        let stderr = child.stderr.take();

        let shared = Arc::new(Mutex::new(StreamBuffer::default()));
        spawn_reader(
            stdout,
            Arc::clone(&shared),
            adb_path.to_path_buf(),
            serial.to_string(),
            snapshot,
        );
        if let Some(stderr) = stderr {
            spawn_stderr_reader(stderr, Arc::clone(&shared));
        }

        Ok(Self {
            child,
            shared,
            session_id,
            serial: serial.to_string(),
        })
    }

    /// Take everything parsed since the last poll and report how the stream is doing.
    pub fn drain(&mut self) -> (Vec<LogEntry>, StreamStatus) {
        let mut buffer = match self.shared.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        // The reader thread sets `finished` *after* flushing its last batch, so a poll can
        // never observe "stopped" while entries are still in flight.
        if buffer.finished && buffer.error.is_empty() {
            if let Ok(Some(status)) = self.child.try_wait() {
                if !status.success() {
                    buffer.error = if buffer.stderr.trim().is_empty() {
                        match status.code() {
                            Some(code) => format!("adb logcat 异常退出（退出码 {code}）"),
                            None => "adb logcat 异常退出（被信号终止）".to_string(),
                        }
                    } else {
                        buffer.stderr.trim().to_string()
                    };
                }
            }
        }

        let pending = std::mem::take(&mut buffer.pending);
        let status = StreamStatus {
            session_id: self.session_id.clone(),
            // A child that exited is reported as stopped even if `finished` flips a poll
            // later; the buffer keeps whatever arrived in the meantime.
            running: !buffer.finished,
            entry_count: 0,
            total_lines: buffer.total_lines,
            unparsed_lines: buffer.unparsed_lines,
            error: buffer.error.clone(),
            // Facets are filled in by the command layer, which owns the session.
            ..Default::default()
        };
        (pending, status)
    }

    /// Give adb a short window to fail. `adb logcat` exits immediately for an
    /// unauthorized / offline device or a dead server, and reporting that as "connected but
    /// empty" would be misleading.
    pub fn startup_error(&mut self) -> Option<String> {
        for _ in 0..8 {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        return None;
                    }
                    let stderr = self
                        .shared
                        .lock()
                        .map(|buffer| buffer.stderr.trim().to_string())
                        .unwrap_or_default();
                    return Some(if stderr.is_empty() {
                        match status.code() {
                            Some(code) => format!("adb logcat 启动失败（退出码 {code}）"),
                            None => "adb logcat 启动失败（进程被终止）".to_string(),
                        }
                    } else {
                        stderr
                    });
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
                Err(_) => return None,
            }
        }
        None
    }

    /// Kill the child. Idempotent: killing an already-exited process is a no-op.
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_reader(
    stdout: std::process::ChildStdout,
    shared: Arc<Mutex<StreamBuffer>>,
    adb_path: std::path::PathBuf,
    serial: String,
    snapshot: String,
) {
    std::thread::spawn(move || {
        let mut parser = match IncrementalParser::new() {
            Ok(parser) => parser,
            Err(e) => {
                finish_with_error(&shared, format!("解析器初始化失败: {e}"));
                return;
            }
        };
        if !snapshot.is_empty() {
            parser.merge_process_snapshot(&snapshot);
        }
        let mut last_snapshot = Instant::now();

        let reader = BufReader::with_capacity(64 * 1024, stdout);
        let mut batch: Vec<LogEntry> = Vec::new();
        let mut since_flush = 0u32;
        let mut last_flush = Instant::now();

        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(e) => {
                    flush(&shared, &mut batch, parser.total_lines(), parser.unparsed_lines());
                    finish_with_error(&shared, format!("读取 adb 输出失败: {e}"));
                    return;
                }
            };
            // Refresh before parsing, so the lines about to be read benefit from it. Only
            // runs while lines are arriving: a silent device needs no name resolution.
            if last_snapshot.elapsed() >= PS_REFRESH {
                let text = adb::process_snapshot(&adb_path, &serial);
                if !text.is_empty() {
                    parser.merge_process_snapshot(&text);
                }
                last_snapshot = Instant::now();
            }
            parser.push_line(&line);
            batch.extend(parser.take_ready());
            since_flush += 1;
            // The time check only runs when a line arrives, which is exactly when there is
            // something new to publish. No lines means nothing to flush.
            if since_flush >= FLUSH_EVERY_LINES || last_flush.elapsed() >= FLUSH_EVERY {
                flush(&shared, &mut batch, parser.total_lines(), parser.unparsed_lines());
                since_flush = 0;
                last_flush = Instant::now();
            }
        }

        let (tail, total_lines, unparsed_lines) = parser.finish();
        batch.extend(tail);
        flush(&shared, &mut batch, total_lines, unparsed_lines);
        if let Ok(mut buffer) = shared.lock() {
            buffer.finished = true;
        }
    });
}

fn spawn_stderr_reader(stderr: std::process::ChildStderr, shared: Arc<Mutex<StreamBuffer>>) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut text = String::new();
        for line in reader.lines().map_while(Result::ok) {
            if text.len() < 4000 {
                text.push_str(&line);
                text.push('\n');
            }
        }
        if let Ok(mut buffer) = shared.lock() {
            buffer.stderr = text;
        }
    });
}

fn flush(shared: &Arc<Mutex<StreamBuffer>>, batch: &mut Vec<LogEntry>, lines: u32, unparsed: u32) {
    let drained = std::mem::take(batch);
    let mut buffer = match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    buffer.pending.extend(drained);
    buffer.total_lines = lines;
    buffer.unparsed_lines = unparsed;
}

fn finish_with_error(shared: &Arc<Mutex<StreamBuffer>>, error: String) {
    let mut buffer = match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if buffer.error.is_empty() {
        buffer.error = error;
    }
    buffer.finished = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn attached_device() -> (std::path::PathBuf, String) {
        let adb_path = crate::adb::locate().expect("应找到 adb");
        let serial = crate::adb::devices()
            .expect("adb devices 应成功")
            .into_iter()
            .find(|d| d.is_ready())
            .expect("需要一台已授权设备")
            .serial;
        (adb_path, serial)
    }

    fn attached_stream(serial: &str) -> DeviceStream {
        let adb_path = crate::adb::locate().expect("应找到 adb");
        let args = crate::adb::logcat_args(serial, &[]);
        DeviceStream::spawn(&adb_path, args, serial, "test-session".to_string())
            .expect("应能启动 adb logcat")
    }

    /// The exact sequence `poll_device_stream` performs: drain the reader buffer, append to
    /// the session, then query. Proves the UI's 800ms tick actually has data to show and
    /// that the match cache does not go stale when entries are appended.
    #[test]
    #[ignore = "需要本机安装 adb 并连接设备"]
    fn poll_append_and_query_round_trip() {
        let (_, serial) = attached_device();
        let mut stream = attached_stream(&serial);
        assert!(stream.startup_error().is_none());

        let mut session = crate::session::LogSession::new(Vec::new());
        let mut appended = 0u32;
        let settle = Instant::now();
        while settle.elapsed() < Duration::from_millis(1500) && appended == 0 {
            std::thread::sleep(Duration::from_millis(150));
            let (pending, _) = stream.drain();
            appended = session.append_all(pending);
        }
        stream.stop();
        assert!(appended > 0, "首次轮询没有可追加的条目");

        let filter = crate::model::FilterSpec::default();
        let first = session.query(&filter, 0, 50).expect("查询应成功");
        assert_eq!(first.total, appended, "条目总数应与追加数一致");
        assert!(first.matched > 0, "无筛选时应有命中");
        assert_eq!(first.entries[0].index, 0, "追加时应从 0 开始编号");
        // Indices must stay contiguous, otherwise the virtual table keys collide.
        for (i, entry) in session.entries.iter().enumerate() {
            assert_eq!(entry.index as usize, i);
        }
    }

    /// The package dimension is only reachable through the pid, so this is the guard that
    /// the reconstruction works against a live device: the `ps` snapshot plus
    /// `am_proc_start` / `Start proc` rows must name a decent share of the replayed lines.
    /// Before this existed, a live capture had no package values at all — only the rare
    /// accessibility foreground event carried one.
    ///
    /// Deliberately split in two, because the two halves fail for unrelated reasons:
    ///   * the live half proves the wiring end to end (capture → parse → `entry.package`);
    ///   * the resolved half proves the *mapping* does not degenerate into a single value.
    /// Diversity is a property of the mapping, not of how chatty the phone happens to be: a
    /// settled device can spend a whole minute printing only `systemui`, and asserting
    /// `> 1` package against live traffic then fails on a correct build. So the second
    /// assertion runs against the device's own `ps` output, which always names dozens of
    /// apps, and checks that each pid keeps its own name instead of collapsing onto one.
    #[test]
    #[ignore = "需要本机安装 adb 并连接设备"]
    fn live_capture_resolves_packages_from_pids() {
        let (adb_path, serial) = attached_device();
        let snapshot = crate::adb::process_snapshot(&adb_path, &serial);
        assert!(!snapshot.trim().is_empty(), "ps 快照不应为空");

        // --- live half: entries that reach the session carry a package -----------------
        let mut stream = attached_stream(&serial);
        assert!(stream.startup_error().is_none());

        let mut session = crate::session::LogSession::new(Vec::new());
        let deadline = Instant::now() + Duration::from_millis(4000);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
            let (pending, _) = stream.drain();
            session.append_all(pending);
            let named = session.entries.iter().filter(|e| !e.package.is_empty()).count();
            if named >= 5 {
                break;
            }
        }
        stream.stop();

        let total = session.entries.len();
        let labelled = session.entries.iter().filter(|e| !e.package.is_empty()).count();
        let mut packages: Vec<&str> = session
            .entries
            .iter()
            .map(|e| e.package.as_str())
            .filter(|p| !p.is_empty())
            .collect();
        packages.sort_unstable();
        packages.dedup();
        assert!(total > 0, "实时抓取没有拿到任何条目");
        assert!(labelled > 0, "{total} 条日志里没有一条解析出包名");

        // --- resolved half: every app pid in the snapshot keeps its own package -------
        // Same code path the reader thread uses: adopt the snapshot, then let each line
        // resolve through `fill`. Synthesising the lines is what makes this deterministic;
        // the *snapshot* is still the device's real, current `ps` output.
        let apps: Vec<(u32, String)> = snapshot
            .lines()
            .filter_map(|line| {
                // `ps -A -o PID,NAME` — two columns; the header and `[kworker/…]` rows drop out.
                let mut cols = line.split_whitespace();
                let pid = cols.next()?.parse::<u32>().ok()?;
                let name = cols.next()?;
                (name.contains('.') && !name.starts_with('[')).then(|| (pid, name.to_string()))
            })
            .collect();
        assert!(apps.len() > 3, "ps 快照里应用进程太少（{} 个），无法验证映射", apps.len());

        let mut parser = crate::logcat::IncrementalParser::new().expect("解析器应能初始化");
        assert!(parser.merge_process_snapshot(&snapshot) > 0, "快照没有被采纳");
        // Three distinct pids → three distinct packages. A mapping that collapsed onto one
        // value shows up here immediately, whatever the phone is doing.
        let probes: Vec<(u32, String)> = apps
            .iter()
            .filter(|(_, name)| name.starts_with("com.") || name.starts_with("cn."))
            .take(3)
            .cloned()
            .collect();
        for (pid, _) in &probes {
            parser.push_line(&format!("11-12 00:00:00.000 {pid} {pid} I Probe: live probe"));
        }
        parser.push_line("11-12 00:00:00.001 0 0 I Probe: sentinel");
        let resolved: Vec<crate::model::LogEntry> = parser.take_ready();

        assert_eq!(resolved.len(), probes.len(), "探针行应有同样多的条目");
        for ((pid, expected), entry) in probes.iter().zip(&resolved) {
            assert_eq!(entry.pid, *pid, "探针行的 pid 解析错了");
            assert_eq!(
                &entry.package, expected,
                "pid {pid} 解析成了 {:?}，应为 {expected} —— 映射共享了同一个名字",
                entry.package
            );
        }

        eprintln!(
            "{total} 条日志中 {labelled} 条带包名，实时看到 {} 个包: {packages:?}；快照验证 {} 个 pid 各自独立",
            packages.len(),
            probes.len()
        );
    }

    /// The property that makes a live capture feel live.
    ///
    /// Regression guard for the "flush only every 128 lines" bug: a phone settling at
    /// ~14 lines/s left the UI (and the counters riding along with the flush) frozen for
    /// ~9 seconds at a time. The probe line is written by the device itself, so this
    /// measures our latency rather than how chatty the phone happens to be.
    #[test]
    #[ignore = "需要本机安装 adb 并连接设备"]
    fn steady_stream_delivers_a_probe_line_within_a_second() {
        let (adb_path, serial) = attached_device();
        let mut stream = attached_stream(&serial);
        assert!(stream.startup_error().is_none(), "adb logcat 启动即失败");

        // Drain the initial ring-buffer burst so it cannot mask the latency under test.
        let settle = Instant::now();
        while settle.elapsed() < Duration::from_millis(1500) {
            std::thread::sleep(Duration::from_millis(120));
            stream.drain();
        }

        let marker = format!(
            "probe-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let status = crate::adb::command(&adb_path)
            .args(["-s", &serial, "shell", "log", "-t", "LogLensProbe", &marker])
            .status()
            .expect("应能向设备写测试日志");
        assert!(status.success(), "adb shell log 失败");

        let mark = Instant::now();
        while mark.elapsed() < Duration::from_secs(4) {
            std::thread::sleep(Duration::from_millis(80));
            let (pending, _) = stream.drain();
            let hit = pending
                .iter()
                .any(|e| e.tag == "LogLensProbe" && e.message.contains(&marker));
            if hit {
                let gap = mark.elapsed();
                stream.stop();
                assert!(
                    gap < Duration::from_millis(1500),
                    "标记日志用了 {gap:?} 才交付，实时性不达标"
                );
                return;
            }
        }
        stream.stop();
        panic!("4 秒内没有收到设备写入的标记日志，读线程可能仍在攒批次");
    }

    /// End-to-end check of the live path against a real phone:
    ///   cd src-tauri && cargo test --lib -- --ignored
    /// A connected-but-idle device still emits a steady trickle of system logs, so 1.5s is
    /// enough to prove the child spawns, stdout is read, lines are parsed and drained.
    #[test]
    #[ignore = "需要本机安装 adb 并连接设备"]
    fn streams_live_lines_from_a_real_device() {
        let adb_path = crate::adb::locate().expect("应找到 adb");
        let serial = crate::adb::devices()
            .expect("adb devices 应成功")
            .into_iter()
            .find(|d| d.is_ready())
            .expect("需要一台已授权设备")
            .serial;

        let args = crate::adb::logcat_args(&serial, &[]);
        let mut stream = DeviceStream::spawn(&adb_path, args, &serial, "test-session".to_string())
            .expect("应能启动 adb logcat");
        assert!(
            stream.startup_error().is_none(),
            "adb logcat 启动即失败，设备状态可能不对"
        );

        std::thread::sleep(Duration::from_millis(1500));
        let (pending, status) = stream.drain();
        stream.stop();

        assert!(status.running, "1.5 秒内流不应结束: {:?}", status.error);
        assert!(
            status.total_lines > 0 || !pending.is_empty(),
            "1.5 秒内没有收到任何日志行"
        );
        // Entries handed out by `drain` must be complete rows, not half-parsed ones.
        for entry in &pending {
            assert!(!entry.raw.is_empty());
            assert_ne!(entry.level, "");
        }
    }
}
