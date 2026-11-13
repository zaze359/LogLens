//! Live device network capture and the device-side proxy guard.
//!
//! Two independent facilities live here:
//!
//! * [`NetworkCapture`] owns a CDP reader thread. It maps a WebView devtools socket
//!   through `adb forward`, waits for a page target, enables `Network` and turns the
//!   event stream into [`NetworkRequest`] rows. It changes **nothing** on the device
//!   except the port forward, which is always removed on stop.
//! * [`ProxyGuard`] owns the global HTTP proxy setting for the MITM route. Setting a
//!   proxy is the one capture action that can leave a device offline if the app crashes,
//!   so the previous value is remembered and restored on stop and in `Drop`.

use crate::cdp::{self, CdpState, WsClient};
use crate::network::NetworkRequest;
use crate::{adb, netprobe};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkCaptureStatus {
    pub session_id: String,
    pub running: bool,
    /// Requests parsed so far (including the ones appended by this poll).
    pub entry_count: u32,
    pub events: u32,
    /// URL of the WebView page currently attached.
    pub target: String,
    pub error: String,
}

#[derive(Default)]
struct CaptureBuffer {
    pending: Vec<NetworkRequest>,
    events: u32,
    target: String,
    finished: bool,
    error: String,
}

/// `date +%z` → seconds east of UTC. CDP `wallTime` is UTC while logcat stamps are
/// device-local, so this offset is what makes the two timelines line up.
pub fn parse_tz_offset(text: &str) -> i32 {
    let value = text.trim().trim_matches(|c: char| !c.is_ascii_digit() && c != '+' && c != '-');
    let value = value.trim_end_matches(|c: char| !c.is_ascii_digit());
    let negative = value.starts_with('-');
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return 0;
    }
    let hh: i32 = digits[..2].parse().unwrap_or(0);
    let mm: i32 = digits[2..4].parse().unwrap_or(0);
    let total = hh * 3600 + mm * 60;
    if negative { -total } else { total }
}

pub fn device_tz_offset(adb_path: &Path, serial: &str) -> i32 {
    parse_tz_offset(&adb::shell_probe(adb_path, serial, "date +%z"))
}

// ---------------------------------------------------------------------------
// Device proxy: used only by the MITM route, never by CDP.
// ---------------------------------------------------------------------------

fn proxy_args(serial: &str, script: &str) -> Vec<String> {
    adb::shell_args(serial, script)
}

#[derive(Debug)]
pub struct ProxyGuard {
    adb_path: PathBuf,
    serial: String,
    original: String,
    applied: String,
    active: bool,
}

impl ProxyGuard {
    /// Read the current value first, set the new one, then read it back — some vendor
    /// ROMs silently ignore `settings put global http_proxy`, and a capture that reports
    /// success while traffic still goes direct is worse than a clear error.
    pub fn apply(adb_path: &Path, serial: &str, proxy: &str) -> Result<Self, String> {
        let proxy = proxy.trim();
        if proxy.is_empty() || !proxy.contains(':') {
            return Err("代理格式应为 主机:端口，例如 10.0.0.2:8080".to_string());
        }
        let current = adb::shell_probe(adb_path, serial, "settings get global http_proxy");
        let original = netprobe::parse_proxy(&current);
        adb::run(
            adb_path,
            &proxy_args(serial, &format!("settings put global http_proxy {proxy}")),
        )
        .map_err(|e| format!("设置设备代理失败: {e}"))?;
        let verify = adb::shell_probe(adb_path, serial, "settings get global http_proxy");
        if netprobe::parse_proxy(&verify) != proxy {
            let _ = Self::restore_value(adb_path, serial, &original);
            return Err("设备拒绝了全局代理设置（部分厂商 ROM 忽略该设置）。请在 系统设置 → WLAN → 当前网络 → 高级 → 代理 中手动填写。".to_string());
        }
        Ok(Self {
            adb_path: adb_path.to_path_buf(),
            serial: serial.to_string(),
            original,
            applied: proxy.to_string(),
            active: true,
        })
    }

    fn restore_value(adb_path: &Path, serial: &str, original: &str) -> Result<(), String> {
        let script = if original.is_empty() {
            "settings delete global http_proxy".to_string()
        } else {
            format!("settings put global http_proxy {original}")
        };
        adb::run(adb_path, &proxy_args(serial, &script)).map(|_| ())
    }

    pub fn applied(&self) -> &str {
        &self.applied
    }

    pub fn original(&self) -> &str {
        &self.original
    }

    pub fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let result = Self::restore_value(&self.adb_path, &self.serial, &self.original);
        self.active = false;
        result
    }
}

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        if self.active {
            if let Err(e) = self.restore() {
                eprintln!("恢复设备代理失败（{}）: {e}", self.serial);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CDP capture
// ---------------------------------------------------------------------------

pub struct NetworkCapture {
    session_id: String,
    serial: String,
    port: u16,
    shared: Arc<Mutex<CaptureBuffer>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn set_error(shared: &Arc<Mutex<CaptureBuffer>>, error: String) {
    if let Ok(mut buffer) = shared.lock() {
        if buffer.error.is_empty() {
            buffer.error = error;
        }
        buffer.finished = true;
    }
}

fn publish(shared: &Arc<Mutex<CaptureBuffer>>, state: &mut CdpState, target: &str) {
    let rows = state.take_completed();
    if rows.is_empty() {
        return;
    }
    if let Ok(mut buffer) = shared.lock() {
        buffer.pending.extend(rows);
        if !target.is_empty() {
            buffer.target = target.to_string();
        }
    }
}

impl NetworkCapture {
    /// Start following WebView network traffic. `socket` comes from the capability
    /// probe (`webview_devtools_remote_<pid>`).
    pub fn start_cdp(
        adb_path: &Path,
        serial: &str,
        socket: &str,
        port: u16,
        offset_secs: i32,
        session_id: String,
    ) -> Result<Self, String> {
        // Fail fast, before a session exists: a stale forward or a dead socket should be
        // reported as "could not start", not as an empty capture that never fills up.
        adb::run(adb_path, &adb::forward_args(serial, port, socket))
            .map_err(|e| format!("建立 adb forward 失败: {e}"))?;
        let version = cdp::version(port).map_err(|e| {
            let _ = adb::run(adb_path, &adb::forward_remove_args(serial, port));
            format!("WebView 调试端口无响应: {e}")
        })?;

        let shared = Arc::new(Mutex::new(CaptureBuffer {
            target: version.package.clone(),
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let adb_path = adb_path.to_path_buf();
        let serial_owned = serial.to_string();
        let thread = std::thread::spawn(move || {
            run_cdp_loop(&adb_path, &serial_owned, port, offset_secs, &thread_shared, &thread_stop);
            // The forward belongs to this capture, even if the loop died early.
            let _ = adb::run(&adb_path, &adb::forward_remove_args(&serial_owned, port));
        });

        Ok(Self {
            session_id,
            serial: serial.to_string(),
            port,
            shared,
            stop,
            thread: Some(thread),
        })
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn drain(&mut self) -> (Vec<NetworkRequest>, NetworkCaptureStatus) {
        let mut buffer = match self.shared.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let pending = std::mem::take(&mut buffer.pending);
        let status = NetworkCaptureStatus {
            session_id: self.session_id.clone(),
            running: !buffer.finished,
            entry_count: 0,
            events: buffer.events,
            target: buffer.target.clone(),
            error: buffer.error.clone(),
        };
        (pending, status)
    }

    /// Signal the reader and wait for it. The reader thread owns the `adb forward`
    /// cleanup (including its early-exit paths), so joining here guarantees the port is
    /// released before the UI reports the capture as stopped.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for NetworkCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run_cdp_loop(
    adb_path: &Path,
    serial: &str,
    port: u16,
    offset_secs: i32,
    shared: &Arc<Mutex<CaptureBuffer>>,
    stop: &Arc<AtomicBool>,
) {
    let mut state = CdpState::new(offset_secs);
    let mut failures = 0u32;
    let mut next_id = 1u64;

    while !stop.load(Ordering::Relaxed) {
        let targets = match cdp::targets(port) {
            Ok(list) => list,
            Err(e) => {
                failures += 1;
                if failures > 20 {
                    set_error(shared, format!("CDP 端口持续不可达: {e}"));
                    return;
                }
                idle_sleep(stop);
                continue;
            }
        };
        let Some(page) = targets
            .iter()
            .find(|t| t.kind == "page" && !t.ws_path.is_empty())
        else {
            // No H5 page open yet. This is normal right after starting a capture.
            idle_sleep(stop);
            continue;
        };

        let target_url = if page.url.is_empty() { page.title.clone() } else { page.url.clone() };
        if let Ok(mut buffer) = shared.lock() {
            buffer.target = target_url.clone();
        }
        let mut ws = match WsClient::connect(port, &page.ws_path) {
            Ok(ws) => ws,
            Err(e) => {
                failures += 1;
                if failures > 20 {
                    set_error(shared, format!("无法连接 WebView 页面: {e}"));
                    return;
                }
                idle_sleep(stop);
                continue;
            }
        };
        next_id += 1;
        if let Err(e) = ws.send_text(&cdp::enable_network_message(next_id)) {
            set_error(shared, format!("启用 Network 域失败: {e}"));
            return;
        }
        failures = 0;

        while !stop.load(Ordering::Relaxed) {
            match ws.recv_text() {
                Ok(None) => break, // page closed / reloaded → re-attach
                Ok(Some(text)) if text.is_empty() => {
                    publish(shared, &mut state, &target_url);
                }
                Ok(Some(text)) => {
                    if let Ok(message) = serde_json::from_str::<Value>(&text) {
                        if state.apply(&message) {
                            if let Ok(mut buffer) = shared.lock() {
                                buffer.events += 1;
                            }
                        }
                    }
                    publish(shared, &mut state, &target_url);
                }
                Err(e) => {
                    // A dropped WebSocket is recoverable; a real protocol error is not.
                    if let Ok(mut buffer) = shared.lock() {
                        if buffer.error.is_empty() {
                            buffer.error = e;
                        }
                    }
                    break;
                }
            }
        }
        publish(shared, &mut state, &target_url);
    }

    let rows = state.flush();
    if !rows.is_empty() {
        if let Ok(mut buffer) = shared.lock() {
            buffer.pending.extend(rows);
        }
    }
    if let Ok(mut buffer) = shared.lock() {
        buffer.finished = true;
    }
    let _ = adb::run(adb_path, &adb::forward_remove_args(serial, port));
}

fn idle_sleep(stop: &Arc<AtomicBool>) {
    for _ in 0..6 {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_timezone_offsets() {
        assert_eq!(parse_tz_offset("+0800"), 8 * 3600);
        assert_eq!(parse_tz_offset("-0530"), -(5 * 3600 + 1800));
        assert_eq!(parse_tz_offset("+0000"), 0);
        assert_eq!(parse_tz_offset(""), 0);
    }

    #[test]
    fn rejects_a_proxy_without_a_port() {
        let adb_path = adb::locate().unwrap_or_else(|| PathBuf::from("/nonexistent/adb"));
        let err = ProxyGuard::apply(&adb_path, "NO_SUCH_DEVICE", "10.0.0.2").unwrap_err();
        assert!(err.contains("主机:端口"));
    }

    #[test]
    fn proxy_scripts_restore_and_delete_correctly() {
        assert_eq!(
            adb::shell_args("S1", "settings get global http_proxy"),
            vec!["-s", "S1", "shell", "settings get global http_proxy"]
        );
        let original = netprobe::parse_proxy("null");
        assert!(original.is_empty(), "null 应视为没有代理，恢复时用 delete");
    }

    // Opt-in: cd src-tauri && cargo test --lib -- --ignored --nocapture
    #[test]
    #[ignore = "需要连接设备且 App 打开了一个 H5 页面"]
    fn captures_real_webview_requests() {
        let adb_path = adb::locate().expect("应找到 adb");
        let serial = adb::devices()
            .expect("adb devices 应成功")
            .into_iter()
            .find(|d| d.is_ready())
            .expect("需要一台已授权设备")
            .serial;
        let packages = netprobe::list_packages(&adb_path, &serial);
        let report = netprobe::probe(&adb_path, &serial, &packages).expect("体检应成功");
        let socket = report.evidence.devtools.first().expect("需要 WebView 调试通道").socket.clone();
        let mut capture = NetworkCapture::start_cdp(
            &adb_path,
            &serial,
            &socket,
            19333,
            device_tz_offset(&adb_path, &serial),
            "test".into(),
        )
        .expect("应能启动 CDP 采集");
        std::thread::sleep(Duration::from_secs(5));
        let (rows, status) = capture.drain();
        capture.stop();
        eprintln!("target={} events={} rows={}", status.target, status.events, rows.len());
        assert!(status.error.is_empty(), "采集出错: {}", status.error);
    }
}
