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
use crate::proxy::ProxyCapture;
use crate::{adb, netprobe};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
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
    /// Every proxy-related key and its value *before* this capture touched the device.
    original: HashMap<String, String>,
    original_display: String,
    applied: String,
    active: bool,
}

/// Delete all proxy keys, force a real "no proxy" state, then put back originals.
///
/// Two Android 15 traps, both reproduced on the target tablet:
/// 1. `settings delete global http_proxy` does not remove AOSP's split
///    `global_http_proxy_host/port` keys.
/// 2. Deleting alone is still not enough: Settings reads back clean, but
///    ConnectivityService keeps the Java-level `http.proxyHost` inherited from
///    Zygote, so every newly *started* app keeps dialling the dead port
///    ("failed to connect to /172.21.33.28 (port 18999)") even after an app
///    restart. Writing the standard no-proxy value `:0` triggers the settings
///    change that actually flushes it.
const NO_PROXY: &str = ":0";

fn restore_script(original: &HashMap<String, String>) -> String {
    let mut script = String::new();
    for key in netprobe::PROXY_KEYS {
        script.push_str(&format!("settings delete global {key} 2>/dev/null; "));
    }
    script.push_str(&format!(
        "settings put global http_proxy {NO_PROXY} 2>/dev/null; "
    ));
    for (key, value) in original {
        if value.is_empty() {
            continue;
        }
        // Values here are host:port, hostnames or port numbers — no shell metacharacters.
        script.push_str(&format!("settings put global {key} '{value}' 2>/dev/null; "));
    }
    script.push_str("true");
    script
}

fn restore_settings(
    adb_path: &Path,
    serial: &str,
    original: &HashMap<String, String>,
) -> Result<(), String> {
    let script = restore_script(original);
    adb::run(adb_path, &proxy_args(serial, &script)).map(|_| ())
}

// ---------------------------------------------------------------------------
// Crash-safe proxy state
//
// A device global proxy is *persistent*, while everything on the desktop is
// volatile. A `Drop` impl only helps on a graceful exit: Cmd+Q, a crash, a
// force-quit, or unplugging the cable before quitting all skip it, leaving the
// phone pointing at a dead port ("failed to connect to /172.21.33.28:18999").
// So every applied proxy is journaled to disk and healed on the next start and
// on every environment probe.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxyMarkerEntry {
    pub adb_path: String,
    pub applied: String,
    pub original: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ProxyMarkerFile {
    version: u8,
    entries: HashMap<String, ProxyMarkerEntry>,
}

fn proxy_state_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("com.zaze.loglens").join("proxy-state.json"))
}

fn read_markers_at(path: &Path) -> ProxyMarkerFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_markers_at(path: &Path, file: &ProxyMarkerFile) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("无法创建配置目录: {e}"))?;
    }
    let text = serde_json::to_string_pretty(file).map_err(|e| format!("序列化代理状态失败: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("写入代理状态失败: {e}"))
}

fn read_proxy_markers() -> ProxyMarkerFile {
    match proxy_state_path() {
        Some(path) => read_markers_at(&path),
        None => ProxyMarkerFile::default(),
    }
}

fn write_proxy_markers(file: &ProxyMarkerFile) -> Result<(), String> {
    let path = proxy_state_path().ok_or_else(|| "无法定位配置目录".to_string())?;
    write_markers_at(&path, file)
}

pub fn mark_proxy_applied(
    adb_path: &Path,
    serial: &str,
    applied: &str,
    original: &HashMap<String, String>,
) {
    let mut file = read_proxy_markers();
    file.version = 1;
    file.entries.insert(
        serial.to_string(),
        ProxyMarkerEntry {
            adb_path: adb_path.to_string_lossy().into_owned(),
            applied: applied.to_string(),
            original: original.clone(),
        },
    );
    if let Err(e) = write_proxy_markers(&file) {
        eprintln!("记录设备代理状态失败: {e}");
    }
}

pub fn clear_proxy_marker(serial: &str) {
    let mut file = read_proxy_markers();
    if file.entries.remove(serial).is_some() {
        let _ = write_proxy_markers(&file);
    }
}

/// Take the marker out of the journal (manual restore command).
pub fn take_proxy_marker(serial: &str) -> Option<ProxyMarkerEntry> {
    let mut file = read_proxy_markers();
    let entry = file.entries.remove(serial)?;
    let _ = write_proxy_markers(&file);
    Some(entry)
}

pub(crate) fn restore_proxy_settings(
    adb_path: &Path,
    serial: &str,
    original: &HashMap<String, String>,
) -> Result<(), String> {
    restore_settings(adb_path, serial, original)
}

/// True when a proxy string points back at this machine. Manual restore uses this
/// to recognise a dead proxy left by a crashed run, without ever deleting a proxy
/// the user (or another tool) configured on the device.
pub fn proxy_points_to_this_machine(proxy: &str) -> bool {
    let Some((host, _)) = proxy.trim().rsplit_once(':') else {
        return false;
    };
    let Ok(ip) = host.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    if ip.is_loopback() {
        return true;
    }
    crate::proxy::local_lan_ip(None)
        .map(|lan| lan == ip)
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealAction {
    /// Device offline / adb unavailable — keep the journal entry for later.
    Keep,
    /// Device already clean — drop the marker.
    ClearMarker,
    /// Somebody else changed the proxy — never delete it.
    LeaveAlone,
    /// Still our proxy — restore the previous values.
    Restore,
}

/// Pure decision, so the two safety rules can be tested without a device:
/// never drop a marker while the device is offline, and never delete a proxy
/// that was put in place by somebody else since our last run.
fn heal_decision(online: bool, current_proxy: &str, applied: &str) -> HealAction {
    if !online {
        return HealAction::Keep;
    }
    if current_proxy.is_empty() {
        return HealAction::ClearMarker;
    }
    if current_proxy != applied {
        return HealAction::LeaveAlone;
    }
    HealAction::Restore
}

/// Undo proxies left behind by a previous run that never got to clean up.
/// Never touches a proxy we did not set, and keeps the journal entry when the
/// device is offline so reconnecting still heals it.
pub fn heal_stale_proxies() -> String {
    match proxy_state_path() {
        Some(path) => heal_stale_proxies_at(&path),
        None => String::new(),
    }
}

fn heal_stale_proxies_at(path: &Path) -> String {
    let mut file = read_markers_at(path);
    if file.entries.is_empty() {
        return String::new();
    }
    let online: Vec<String> = adb::devices()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.is_ready())
        .map(|d| d.serial)
        .collect();
    let mut notes = Vec::new();
    let mut remaining = HashMap::new();
    for (serial, entry) in std::mem::take(&mut file.entries) {
        let recorded = PathBuf::from(&entry.adb_path);
        let adb_path = if recorded.exists() {
            recorded
        } else {
            match adb::locate() {
                Some(path) => path,
                None => {
                    remaining.insert(serial, entry);
                    continue;
                }
            }
        };
        let is_online = online.contains(&serial);
        let current_proxy = if is_online {
            // An offline device returns "" here because adb failed, NOT because the
            // proxy is gone. `heal_decision` keeps the marker in that case.
            netprobe::proxy_from_settings(&adb::shell_probe(
                &adb_path,
                &serial,
                &netprobe::proxy_settings_script(),
            ))
        } else {
            String::new()
        };
        let action = heal_decision(is_online, &current_proxy, &entry.applied);
        match action {
            HealAction::Keep => {
                remaining.insert(serial, entry);
            }
            HealAction::LeaveAlone => {
                notes.push(format!("{serial}: 代理已由其他程序改写，保持原样"));
            }
            HealAction::ClearMarker | HealAction::Restore => {
                // Even when Settings already reads clean, ConnectivityService can
                // still hand the old proxy to newly forked app processes. Always
                // run the restore — it writes `:0` to force a real refresh.
                let label = if action == HealAction::Restore {
                    format!("已自动清理上次残留的代理 {current_proxy}")
                } else {
                    "上次采集的代理已清除，已刷新系统代理状态".to_string()
                };
                match restore_settings(&adb_path, &serial, &entry.original) {
                    Ok(()) => notes.push(format!("{serial}: {label}")),
                    Err(e) => {
                        notes.push(format!("{serial}: 暂无法清理: {e}"));
                        remaining.insert(serial, entry);
                    }
                }
            }
        }
    }
    file.entries = remaining;
    let _ = write_markers_at(path, &file);
    notes.join("；")
}

impl ProxyGuard {
    /// Capture the whole proxy setting, apply ours, then verify. Some vendor ROMs ignore
    /// `settings put global http_proxy`, and a capture reporting success while traffic
    /// still goes direct is worse than a clear error.
    pub fn apply(adb_path: &Path, serial: &str, proxy: &str) -> Result<Self, String> {
        let proxy = proxy.trim();
        let (host, port) = proxy
            .rsplit_once(':')
            .ok_or_else(|| "代理格式应为 主机:端口，例如 10.0.0.2:8080".to_string())?;
        if host.is_empty() || port.parse::<u16>().is_err() {
            return Err("代理格式应为 主机:端口，例如 10.0.0.2:8080".to_string());
        }

        let script = netprobe::proxy_settings_script();
        let before = adb::shell_probe(adb_path, serial, &script);
        let original = netprobe::parse_proxy_settings(&before);
        let original_display = netprobe::proxy_from_settings(&before);

        adb::run(
            adb_path,
            &proxy_args(serial, &format!("settings put global http_proxy {proxy}")),
        )
        .map_err(|e| format!("设置设备代理失败: {e}"))?;

        let after_text = adb::shell_probe(adb_path, serial, &script);
        let after = netprobe::parse_proxy_settings(&after_text);
        // Android 15 stores the value split into host/port rather than as `http_proxy`.
        let applied_ok = after.get("http_proxy").map(|v| v == proxy).unwrap_or(false)
            || (after
                .get("global_http_proxy_host")
                .map(|v| v == host)
                .unwrap_or(false)
                && after
                    .get("global_http_proxy_port")
                    .map(|v| v == port)
                    .unwrap_or(false));
        if !applied_ok {
            let _ = restore_settings(adb_path, serial, &original);
            return Err("设备拒绝了全局代理设置（部分厂商 ROM 忽略该设置）。请在 系统设置 → WLAN → 当前网络 → 高级 → 代理 中手动填写。".to_string());
        }
        // Journal *before* returning: if the process is killed a millisecond later,
        // the next start must still know this device needs undoing.
        mark_proxy_applied(adb_path, serial, proxy, &original);
        Ok(Self {
            adb_path: adb_path.to_path_buf(),
            serial: serial.to_string(),
            original,
            original_display,
            applied: proxy.to_string(),
            active: true,
        })
    }

    pub fn applied(&self) -> &str {
        &self.applied
    }

    pub fn original(&self) -> &str {
        &self.original_display
    }

    pub fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let result = restore_settings(&self.adb_path, &self.serial, &self.original);
        if result.is_ok() {
            clear_proxy_marker(&self.serial);
        }
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

/// One capture channel, either source. The command layer and the session store only
/// deal with this enum, so adding WebView and native traffic to the *same* table needs no
/// changes to polling, stopping or cleanup.
pub enum AnyCapture {
    Cdp(NetworkCapture),
    Proxy(ProxyCapture),
}

impl AnyCapture {
    pub fn drain(&mut self) -> (Vec<NetworkRequest>, NetworkCaptureStatus) {
        match self {
            AnyCapture::Cdp(c) => c.drain(),
            AnyCapture::Proxy(c) => c.drain(),
        }
    }

    pub fn stop(&mut self) {
        match self {
            AnyCapture::Cdp(c) => c.stop(),
            AnyCapture::Proxy(c) => c.stop(),
        }
    }

    pub fn serial(&self) -> &str {
        match self {
            AnyCapture::Cdp(c) => c.serial(),
            AnyCapture::Proxy(c) => c.serial(),
        }
    }

    pub fn port(&self) -> u16 {
        match self {
            AnyCapture::Cdp(c) => c.port(),
            AnyCapture::Proxy(c) => c.port(),
        }
    }

    /// A device has exactly one global proxy. H5 (CDP) and native capture may run
    /// together, but two native captures cannot — even on different ports, because
    /// only one of them can own the device setting at a time.
    pub fn is_proxy(&self) -> bool {
        matches!(self, AnyCapture::Proxy(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_journal_round_trips_and_survives_a_corrupt_file() {
        // Use a temp file so the test never touches the real user config dir.
        let path = std::env::temp_dir().join(format!(
            "loglens-proxy-journal-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut file = ProxyMarkerFile::default();
        file.version = 1;
        let mut original = HashMap::new();
        original.insert("http_proxy".to_string(), String::new());
        file.entries.insert(
            "SERIAL1".to_string(),
            ProxyMarkerEntry {
                adb_path: "/path/to/adb".into(),
                applied: "172.21.33.28:18999".into(),
                original,
            },
        );
        write_markers_at(&path, &file).unwrap();
        let loaded = read_markers_at(&path);
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries["SERIAL1"].applied, "172.21.33.28:18999");
        assert!(loaded.entries["SERIAL1"].original.contains_key("http_proxy"));

        std::fs::write(&path, "{ not json").unwrap();
        assert!(
            read_markers_at(&path).entries.is_empty(),
            "损坏的 journal 不能让应用误判或崩溃"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn heal_never_drops_a_marker_for_an_offline_device() {
        // This is the exact bug that strands a device: an empty `settings` read
        // while offline means "adb failed", not "proxy cleared".
        assert_eq!(
            heal_decision(false, "", "172.21.33.28:18999"),
            HealAction::Keep
        );
        assert_eq!(
            heal_decision(true, "", "172.21.33.28:18999"),
            HealAction::ClearMarker
        );
        assert_eq!(
            heal_decision(true, "172.21.33.28:18999", "172.21.33.28:18999"),
            HealAction::Restore
        );
        assert_eq!(
            heal_decision(true, "10.0.0.9:8080", "172.21.33.28:18999"),
            HealAction::LeaveAlone
        );
    }

    #[test]
    fn split_proxy_keys_are_recognised_as_the_proxy_we_applied() {
        let text = "http_proxy=null\nglobal_http_proxy_host=172.21.33.28\nglobal_http_proxy_port=18999\n";
        assert_eq!(
            netprobe::proxy_from_settings(text),
            "172.21.33.28:18999"
        );
        assert_eq!(
            heal_decision(
                true,
                &netprobe::proxy_from_settings(text),
                "172.21.33.28:18999"
            ),
            HealAction::Restore
        );
    }

    #[test]
    fn manual_restore_recognises_a_loopback_proxy_as_ours() {
        assert!(proxy_points_to_this_machine("127.0.0.1:18999"));
        assert!(!proxy_points_to_this_machine("not-a-host:18999"));
        assert!(!proxy_points_to_this_machine("garbage"));
    }

    /// Reproduces the reported failure end to end: a previous run set the device
    /// proxy and died without restoring it. The next start must heal it from the
    /// journal, with no in-memory guard involved.
    #[test]
    #[ignore = "需要连接设备并临时改变其全局代理设置"]
    fn heals_a_dead_proxy_left_by_a_crashed_run() {
        let adb_path = adb::locate().expect("应找到 adb");
        let serial = adb::devices()
            .expect("adb devices 应成功")
            .into_iter()
            .find(|d| d.is_ready())
            .expect("需要已授权设备")
            .serial;
        let before = adb::shell_probe(&adb_path, &serial, &netprobe::proxy_settings_script());
        assert!(
            netprobe::proxy_from_settings(&before).is_empty(),
            "设备当前已有代理，跳过以免打断正在进行的采集"
        );
        let original = netprobe::parse_proxy_settings(&before);

        let journal = std::env::temp_dir().join(format!(
            "loglens-heal-journal-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&journal);

        // 1. Leave the dead proxy exactly the way a crashed run would.
        let proxy = format!(
            "{}:18999",
            crate::proxy::local_lan_ip(None).expect("需要本机 LAN 地址")
        );
        adb::run(
            &adb_path,
            &proxy_args(&serial, &format!("settings put global http_proxy {proxy}")),
        )
        .expect("应能设置测试代理");

        // 2. Write the journal as `ProxyGuard::apply` would, then "crash".
        let mut file = ProxyMarkerFile::default();
        file.version = 1;
        file.entries.insert(
            serial.clone(),
            ProxyMarkerEntry {
                adb_path: adb_path.to_string_lossy().into_owned(),
                applied: proxy,
                original: original.clone(),
            },
        );
        write_markers_at(&journal, &file).unwrap();

        // 3. Simulate the next launch.
        let note = heal_stale_proxies_at(&journal);
        let after = adb::shell_probe(&adb_path, &serial, &netprobe::proxy_settings_script());
        // The real regression: an empty settings read is NOT proof the effective
        // proxy is gone. Only an explicit `:0` makes ConnectivityService flush it.
        let effective = adb::shell_probe(&adb_path, &serial, "settings get global http_proxy");
        // Clean up before asserting so a failed assertion never strands the device.
        let _ = restore_settings(&adb_path, &serial, &original);
        let journal_after = read_markers_at(&journal);
        let _ = std::fs::remove_file(&journal);

        assert_eq!(
            effective.trim(),
            ":0",
            "必须显式写入 :0 才会刷新系统实际生效的代理，否则新起的 App 仍会连死端口"
        );
        assert!(
            netprobe::proxy_from_settings(&after).is_empty(),
            "启动自愈应清掉死代理，note={note}, after={after}"
        );
        assert!(note.contains("已自动清理"), "自愈结果应可见: {note}");
        assert!(journal_after.entries.is_empty(), "成功后应删除 journal 条目");
        eprintln!("自愈结果: {note}");
    }

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

        // AOSP splits http_proxy into host/port keys: restoring must delete *all* of them.
        let script = restore_script(&HashMap::new());
        for key in netprobe::PROXY_KEYS {
            assert!(
                script.contains(&format!("settings delete global {key}")),
                "缺少 {key} 的清理"
            );
        }
        assert!(
            script.contains("settings put global http_proxy :0"),
            "仅 delete 不会清掉系统实际生效的代理，必须显式写 :0"
        );
        assert_eq!(
            script.matches("settings put").count(),
            1,
            "原值为空时只应写 :0，不应写回任何旧代理"
        );
        assert!(script.ends_with("true"));
    }

    #[test]
    fn the_explicit_no_proxy_value_reads_back_as_no_proxy() {
        assert_eq!(netprobe::parse_proxy(":0"), "");
        assert!(netprobe::proxy_from_settings(
            "http_proxy=:0\nglobal_http_proxy_host=null\nglobal_http_proxy_port=null\n"
        )
        .is_empty());
    }

    #[test]
    fn restore_script_puts_back_only_the_original_values() {
        let mut original = HashMap::new();
        original.insert("http_proxy".to_string(), "10.0.0.2:8080".to_string());
        original.insert("global_http_proxy_port".to_string(), String::new());
        let script = restore_script(&original);
        assert!(script.contains("settings put global http_proxy '10.0.0.2:8080'"));
        assert!(!script.contains("settings put global global_http_proxy_port"));
        // Deletes must come first, or a stale split key survives the put.
        let delete_at = script.find("settings delete global global_http_proxy_port").unwrap();
        let put_at = script
            .find("settings put global http_proxy '10.0.0.2:8080'")
            .unwrap();
        assert!(delete_at < put_at, "必须先删再恢复，否则拆出的键会残留");
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
