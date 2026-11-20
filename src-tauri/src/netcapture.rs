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
use std::collections::{HashMap, HashSet};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
    /// The proxy the device had *before* this capture. Only shown in the UI: restore always
    /// clears the proxy instead of writing this value back.
    original_display: String,
    applied: String,
    /// Distinguishes this application from a later capture on the same device, so a
    /// delayed restore retry cannot undo the newer capture's proxy.
    generation: u64,
    active: bool,
}

/// Delete all proxy keys and force a real "no proxy" state.
///
/// Stopping a capture always leaves the device on a direct connection. It must not put back
/// what was there before the capture: that value is often a dead endpoint from an earlier
/// run (this machine's LAN IP changes between sessions), so restoring it keeps the phone
/// dialling a closed port and makes "stop" look like it did nothing.
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

/// Build the shell script that points the device at our listener.
///
/// Mirror `restore_script`: delete every proxy key (a stale split host/port can keep
/// dialling the *previous* capture's dead port), write the standard no-proxy value to
/// force ConnectivityService to refresh, then set both the legacy `http_proxy` and the
/// Android 15 split host/port keys. Without this, a capture can report success while
/// traffic still follows an old setting.
fn apply_script(proxy: &str, host: &str, port: &str) -> String {
    let mut script = String::new();
    for key in netprobe::PROXY_KEYS {
        script.push_str(&format!("settings delete global {key} 2>/dev/null; "));
    }
    // The value change to `:0` is what actually flushes the inherited Java-level proxy.
    script.push_str(&format!(
        "settings put global http_proxy {NO_PROXY} 2>/dev/null; "
    ));
    script.push_str(&format!(
        "settings put global http_proxy '{proxy}' 2>/dev/null; "
    ));
    script.push_str(&format!(
        "settings put global global_http_proxy_host '{host}' 2>/dev/null; "
    ));
    script.push_str(&format!(
        "settings put global global_http_proxy_port '{port}' 2>/dev/null; "
    ));
    script.push_str("true");
    script
}

fn restore_script() -> String {
    let mut script = String::new();
    for key in netprobe::PROXY_KEYS {
        script.push_str(&format!("settings delete global {key} 2>/dev/null; "));
    }
    script.push_str(&format!(
        "settings put global http_proxy {NO_PROXY} 2>/dev/null; "
    ));
    script.push_str("true");
    script
}

fn restore_settings(adb_path: &Path, serial: &str) -> Result<(), String> {
    let script = restore_script();
    adb::run(adb_path, &proxy_args(serial, &script))
        .map_err(|e| format!("清空设备代理失败: {e}"))?;
    // The script ends with `true` so one missing key cannot abort the rest; that also
    // means adb's exit code proves nothing. Read the values back, otherwise a device
    // that ignored the write would be reported as restored and its journal deleted.
    let after_text = adb::run(
        adb_path,
        &proxy_args(serial, &netprobe::proxy_settings_script()),
    )
    .map_err(|e| format!("恢复后读取设备代理失败: {e}"))?;
    let actual = netprobe::proxy_from_settings(&after_text);
    if !actual.is_empty() {
        return Err(format!("恢复后设备代理仍为 {actual}（期望直连）"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Restore retry
//
// A restore can fail for a reason that has nothing to do with the proxy: the
// user switched devices, the cable was pulled, or adb dropped the serial for a
// moment. Printing one line and giving up leaves the tablet dialling a dead
// port, so keep the journal entry and retry in the background until the device
// comes back. A generation counter makes sure an old retry never undoes a proxy
// that a *new* capture has just applied to the same device.
// ---------------------------------------------------------------------------

const RESTORE_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const RESTORE_RETRY_WINDOW: Duration = Duration::from_secs(600);

fn proxy_generations() -> &'static Mutex<HashMap<String, u64>> {
    static GENERATIONS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    GENERATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_proxy_generation(serial: &str) -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let generation = SEQ.fetch_add(1, Ordering::Relaxed);
    proxy_generations()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(serial.to_string(), generation);
    generation
}

fn proxy_generation(serial: &str) -> Option<u64> {
    proxy_generations()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(serial)
        .copied()
}

fn forget_proxy_generation(serial: &str, generation: u64) {
    let mut generations = proxy_generations()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if generations.get(serial) == Some(&generation) {
        generations.remove(serial);
    }
}

fn restore_retries() -> &'static Mutex<HashSet<String>> {
    static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Keep trying to clear the device proxy after the first attempt failed.
///
/// `generation` is `None` for a heal of a previous run: that retry must give up as
/// soon as any new capture takes ownership of the device.
fn schedule_restore_retry(adb_path: &Path, serial: &str, generation: Option<u64>) {
    let key = match generation {
        Some(generation) => format!("{serial}#{generation}"),
        None => format!("{serial}#heal"),
    };
    {
        // `restore` runs again from `Drop` in some paths; one thread per device/attempt
        // is enough.
        let mut active = restore_retries()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !active.insert(key.clone()) {
            return;
        }
    }
    let recorded_adb = adb_path.to_path_buf();
    let serial = serial.to_string();
    std::thread::spawn(move || {
        let deadline = Instant::now() + RESTORE_RETRY_WINDOW;
        while Instant::now() < deadline {
            std::thread::sleep(RESTORE_RETRY_INTERVAL);
            let superseded = match generation {
                Some(generation) => proxy_generation(&serial) != Some(generation),
                // A heal retry is stale the moment a new capture owns the device.
                None => proxy_generation(&serial).is_some(),
            };
            if superseded {
                // Do not even log: a new capture may legitimately have re-applied a proxy.
                break;
            }
            let adb = if recorded_adb.exists() {
                recorded_adb.clone()
            } else if let Some(path) = adb::locate() {
                path
            } else {
                continue;
            };
            let online = adb::devices()
                .map(|devices| {
                    devices
                        .iter()
                        .any(|d| d.serial == serial && d.is_ready())
                })
                .unwrap_or(false);
            if !online {
                continue;
            }
            match restore_settings(&adb, &serial) {
                Ok(()) => {
                    clear_proxy_marker(&serial);
                    if let Some(generation) = generation {
                        forget_proxy_generation(&serial, generation);
                    }
                    crate::error_log::flow_echo(
                        "proxy.restore",
                        "设备重新上线，已清空设备代理",
                        &serial,
                    );
                    break;
                }
                Err(e) => {
                    crate::error_log::flow_echo(
                        "proxy.restore",
                        "补做代理清理失败，稍后重试",
                        &format!(
                            "serial={serial} retry={}s error={e}",
                            RESTORE_RETRY_INTERVAL.as_secs()
                        ),
                    );
                }
            }
        }
        restore_retries()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key);
    });
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

pub fn mark_proxy_applied(adb_path: &Path, serial: &str) -> Result<(), String> {
    let mut file = read_proxy_markers();
    file.version = 1;
    file.entries.insert(
        serial.to_string(),
        ProxyMarkerEntry {
            adb_path: adb_path.to_string_lossy().into_owned(),
        },
    );
    write_proxy_markers(&file)
}

pub fn clear_proxy_marker(serial: &str) {
    let mut file = read_proxy_markers();
    if file.entries.remove(serial).is_some() {
        let _ = write_proxy_markers(&file);
    }
}

pub(crate) fn restore_proxy_settings(adb_path: &Path, serial: &str) -> Result<(), String> {
    restore_settings(adb_path, serial)
}

/// Public entry for the manual restore command: retry in the background after a failure
/// instead of dropping the journal entry on the floor.
pub(crate) fn schedule_proxy_restore_retry(adb_path: &Path, serial: &str) {
    schedule_restore_retry(adb_path, serial, None);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealAction {
    /// Device offline / adb unavailable — keep the journal entry for later.
    Keep,
    /// Device already clean — drop the marker.
    ClearMarker,
    /// A proxy is present — clear it.
    Restore,
    /// A capture in this process owns the proxy: not a leftover, leave it running.
    Skip,
}

/// Pure decision, so the safety rules can be tested without a device: never drop a marker
/// while the device is offline, and never touch a proxy this process is *currently* using.
fn heal_decision(online: bool, live: bool, current_proxy: &str) -> HealAction {
    if !online {
        return HealAction::Keep;
    }
    // Healing exists to undo a *previous run's* leftovers. A capture running right now owns
    // this value, and clearing it would silently stop that capture mid-flight — exactly
    // what happens when the panel's capability probe calls this while collecting.
    if live {
        return HealAction::Skip;
    }
    if current_proxy.is_empty() {
        return HealAction::ClearMarker;
    }
    HealAction::Restore
}

/// Clear proxies left behind by a previous run that never got to clean up, and keep the
/// journal entry when the device is offline so reconnecting still heals it.
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
        let action = heal_decision(
            is_online,
            // An active guard registers a generation for its serial; that is the cheapest
            // honest answer to "is this proxy ours *right now*".
            proxy_generation(&serial).is_some(),
            &current_proxy,
        );
        match action {
            HealAction::Skip => {
                // Keep the journal: the guard owns it and will clear it on restore.
                remaining.insert(serial, entry);
            }
            HealAction::Keep => {
                // The device is offline right now. Retry while the app runs instead of
                // only when it is next started; reconnect can happen minutes later.
                schedule_restore_retry(&adb_path, &serial, None);
                remaining.insert(serial, entry);
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
                match restore_settings(&adb_path, &serial) {
                    Ok(()) => notes.push(format!("{serial}: {label}")),
                    Err(e) => {
                        notes.push(format!("{serial}: 暂无法清理: {e}"));
                        schedule_restore_retry(&adb_path, &serial, None);
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
        let original_display = netprobe::proxy_from_settings(&before);

        crate::error_log::record_flow(
            "proxy.apply",
            "设置设备代理",
            &format!("serial={serial} proxy={proxy} before={original_display}"),
        );
        adb::run(adb_path, &proxy_args(serial, &apply_script(proxy, host, port)))
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
            let _ = restore_settings(adb_path, serial);
            crate::error_log::record_flow(
                "proxy.apply",
                "设备代理设置未被接受",
                &format!("serial={serial} proxy={proxy}"),
            );
            return Err("设备拒绝了全局代理设置（部分厂商 ROM 忽略该设置）。请在 系统设置 → WLAN → 当前网络 → 高级 → 代理 中手动填写。".to_string());
        }
        crate::error_log::record_flow(
            "proxy.apply",
            "设备代理已生效",
            &format!(
                "serial={serial} proxy={proxy} actual={} split_host={} split_port={} note=已运行进程需重启才会走代理",
                netprobe::proxy_from_settings(&after_text),
                after.get("global_http_proxy_host").cloned().unwrap_or_default(),
                after.get("global_http_proxy_port").cloned().unwrap_or_default()
            ),
        );
        // Journal *before* returning: if the process is killed a millisecond later,
        // the next start must still know this device needs undoing. If the journal
        // cannot be persisted, refuse to keep the device proxy: otherwise a crash or
        // kill leaves the tablet pointed at a dead port with no way to heal it.
        if let Err(e) = mark_proxy_applied(adb_path, serial) {
            let restored = restore_settings(adb_path, serial);
            return Err(match restored {
                Ok(()) => format!("无法记录代理状态，已清空设备代理: {e}"),
                Err(restore_error) => {
                    format!("无法记录代理状态（{e}），且清空设备代理失败: {restore_error}")
                }
            });
        }
        Ok(Self {
            adb_path: adb_path.to_path_buf(),
            serial: serial.to_string(),
            original_display,
            applied: proxy.to_string(),
            generation: next_proxy_generation(serial),
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
        match restore_settings(&self.adb_path, &self.serial) {
            Ok(()) => {
                clear_proxy_marker(&self.serial);
                forget_proxy_generation(&self.serial, self.generation);
                self.active = false;
                Ok(())
            }
            Err(e) => {
                // Keep the journal entry and hand the job to a background retry: the
                // device may simply be absent right now (switched away, cable pulled).
                schedule_restore_retry(&self.adb_path, &self.serial, Some(self.generation));
                // The retry owns the restore now; `Drop` must not fire a second attempt.
                self.active = false;
                Err(e)
            }
        }
    }
}

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        if self.active {
            if let Err(e) = self.restore() {
                crate::error_log::flow_echo(
                    "proxy.restore",
                    "设备代理恢复失败",
                    &format!("serial={} error={e}", self.serial),
                );
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
    shared: Arc<Mutex<CaptureBuffer>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn set_error(shared: &Arc<Mutex<CaptureBuffer>>, error: String) {
    // The UI shows `status.error`, but "nothing on screen" reports rarely mention it.
    // A flow checkpoint keeps the last reader failure next to the poll counters.
    crate::error_log::record_flow("capture.cdp.error", "CDP 读取器报错", &error);
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

/// Does this `adb` failure mean "the local port is taken" rather than "the device is
/// unreachable"? Only a port conflict is worth retrying on another number.
fn looks_like_port_conflict(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("address already in use")
        || lower.contains("cannot bind")
        || lower.contains("could not bind")
}

/// Ask the OS for a free loopback port, then release it for `adb forward` to claim.
///
/// Inherently a small race — nothing holds the number between the probe and `adb` — but
/// `adb` binds the local side itself, so handing over a number is the only option. The
/// alternative (one fixed port) fails outright whenever a previous forward still owns it.
fn pick_forward_port(taken: u16) -> Option<u16> {
    for _ in 0..8 {
        let probe = TcpListener::bind(("127.0.0.1", 0)).ok()?;
        let port = probe.local_addr().ok()?.port();
        drop(probe);
        // `adb` needs an unprivileged port; 0 would mean "any" again.
        if port != taken && port > 0 {
            return Some(port);
        }
    }
    None
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
        let mut port = port;
        if let Err(first) = adb::run(adb_path, &adb::forward_args(serial, port, socket)) {
            // The local half of `adb forward` is a fixed number by default, and it stays
            // claimed while any capture — or a forward left by a previous run — uses it.
            // A second device must not be locked out by that, so retry once on a port the
            // OS says is free. Only port conflicts are retried: a missing socket or an
            // offline device would just fail again with a confusing second message.
            let alt = if looks_like_port_conflict(&first) {
                pick_forward_port(port)
            } else {
                None
            };
            let Some(alt) = alt else {
                return Err(format!("建立 adb forward 失败: {first}"));
            };
            adb::run(adb_path, &adb::forward_args(serial, alt, socket)).map_err(|second| {
                format!("建立 adb forward 失败: {second}（tcp:{port} 与 tcp:{alt} 均不可用）")
            })?;
            eprintln!("[cdp] 本地端口 tcp:{port} 不可用，改用 tcp:{alt}");
            crate::error_log::record_flow(
                "capture.cdp.forward",
                "改用备用转发端口",
                &format!("serial={serial} preferred={port} actual={alt}"),
            );
            port = alt;
        }
        let version = cdp::version(port).map_err(|e| {
            let _ = adb::run(adb_path, &adb::forward_remove_args(serial, port));
            format!("WebView 调试端口无响应: {e}")
        })?;
        crate::error_log::record_flow(
            "capture.cdp.start",
            "CDP 调试端口就绪",
            &format!("id={session_id} serial={serial} port={port} package={}", version.package),
        );

        let shared = Arc::new(Mutex::new(CaptureBuffer {
            target: version.package.clone(),
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let adb_path = adb_path.to_path_buf();
        let serial_owned = serial.to_string();
        let thread_session = session_id.clone();
        let thread = std::thread::spawn(move || {
            run_cdp_loop(
                &adb_path,
                &serial_owned,
                port,
                offset_secs,
                &thread_shared,
                &thread_stop,
                &thread_session,
            );
            // The forward belongs to this capture, even if the loop died early.
            let _ = adb::run(&adb_path, &adb::forward_remove_args(&serial_owned, port));
        });

        Ok(Self {
            session_id,
            serial: serial.to_string(),
            shared,
            stop,
            thread: Some(thread),
        })
    }

    pub fn serial(&self) -> &str {
        &self.serial
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

    /// Drop rows the UI has not polled yet, mirroring the table being cleared.
    pub fn clear(&mut self) {
        if let Ok(mut buffer) = self.shared.lock() {
            buffer.pending.clear();
        }
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
    session_id: &str,
) {
    let mut state = CdpState::new(offset_secs);
    let mut failures = 0u32;
    let mut next_id = 1u64;
    let mut reported_no_page = false;

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
            // No H5 page open yet. This is normal right after starting a capture, but it
            // is also the most common reason for an empty table — record it once.
            if !reported_no_page {
                reported_no_page = true;
                crate::error_log::record_flow(
                    "capture.cdp.attach",
                    "暂无可附加的 H5 页面",
                    &format!("id={session_id} serial={serial} port={port}"),
                );
            }
            idle_sleep(stop);
            continue;
        };
        reported_no_page = false;
        crate::error_log::record_flow(
            "capture.cdp.attach",
            "已附加 H5 页面",
            &format!("id={session_id} serial={serial} port={port}"),
        );

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
                    crate::error_log::record_flow("capture.cdp.socket", "WebSocket 断开", &e);
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
    let flushed = rows.len();
    if !rows.is_empty() {
        if let Ok(mut buffer) = shared.lock() {
            buffer.pending.extend(rows);
        }
    }
    crate::error_log::record_flow(
        "capture.cdp.end",
        "CDP 读取结束",
        &format!("id={session_id} serial={serial} flushed={flushed}"),
    );
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

    pub fn stop(&mut self) -> Result<(), String> {
        match self {
            AnyCapture::Cdp(c) => {
                c.stop();
                Ok(())
            }
            AnyCapture::Proxy(c) => c.stop(),
        }
    }

    pub fn clear(&mut self) {
        match self {
            AnyCapture::Cdp(c) => c.clear(),
            AnyCapture::Proxy(c) => c.clear(),
        }
    }

    pub fn serial(&self) -> &str {
        match self {
            AnyCapture::Cdp(c) => c.serial(),
            AnyCapture::Proxy(c) => c.serial(),
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
        file.entries.insert(
            "SERIAL1".to_string(),
            ProxyMarkerEntry {
                adb_path: "/path/to/adb".into(),
            },
        );
        write_markers_at(&path, &file).unwrap();
        let loaded = read_markers_at(&path);
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries["SERIAL1"].adb_path, "/path/to/adb");

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
        assert_eq!(heal_decision(false, false, ""), HealAction::Keep);
        assert_eq!(heal_decision(true, false, ""), HealAction::ClearMarker);
        assert_eq!(
            heal_decision(true, false, "172.21.33.28:18999"),
            HealAction::Restore
        );
        // Unconditional: any proxy left on the device gets cleared, whoever set it.
        assert_eq!(
            heal_decision(true, false, "10.0.0.9:8080"),
            HealAction::Restore
        );
        // A capture running right now owns the value; healing must not interrupt it.
        assert_eq!(
            heal_decision(true, true, "172.21.33.28:18999"),
            HealAction::Skip
        );
    }

    /// The regression behind "采集运行中但一条请求都没有": the panel's capability probe
    /// calls the healer, which used to clear the proxy under a running capture.
    #[test]
    fn heal_never_touches_a_proxy_a_running_capture_owns() {
        // Same inputs as the "clear the leftover" case above — the only difference is
        // that a capture in this process still holds the device.
        assert_eq!(
            heal_decision(true, true, "172.21.33.28:18999"),
            HealAction::Skip
        );
        assert_eq!(
            heal_decision(true, true, ""),
            HealAction::Skip,
            "正在采集时连 marker 都不能清，否则退出时就没法补做清理了"
        );
        // Offline still means "cannot judge": the marker must survive either way.
        assert_eq!(heal_decision(false, true, ""), HealAction::Keep);
    }

    #[test]
    fn split_proxy_keys_are_recognised_as_a_proxy_to_clear() {
        let text = "http_proxy=null\nglobal_http_proxy_host=172.21.33.28\nglobal_http_proxy_port=18999\n";
        assert_eq!(
            netprobe::proxy_from_settings(text),
            "172.21.33.28:18999"
        );
        assert_eq!(
            heal_decision(true, false, &netprobe::proxy_from_settings(text)),
            HealAction::Restore
        );
    }

    /// Reproduces the reported failure end to end: a previous run set the device
    /// proxy and died without clearing it. The next start must heal it from the
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
        let _ = restore_settings(&adb_path, &serial);
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
        let script = restore_script();
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
            "停止采集必须回到直连，不能写回任何旧代理"
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
    fn apply_script_clears_stale_keys_and_sets_both_proxy_forms() {
        let script = apply_script("10.0.0.2:8899", "10.0.0.2", "8899");
        let delete_at = script
            .find("settings delete global global_http_proxy_host")
            .unwrap();
        let no_proxy_at = script.find("settings put global http_proxy :0").unwrap();
        let put_at = script
            .find("settings put global http_proxy '10.0.0.2:8899'")
            .unwrap();
        assert!(delete_at < no_proxy_at && no_proxy_at < put_at);
        assert!(script.contains("settings put global global_http_proxy_host '10.0.0.2'"));
        assert!(script.contains("settings put global global_http_proxy_port '8899'"));
        assert!(script.ends_with("true"));
    }

    #[test]
    fn restore_script_never_writes_a_previous_proxy_back() {
        // The exact reported failure: the device still had a dead endpoint from an
        // earlier run (old LAN IP), and stopping the capture put that value back.
        let script = restore_script();
        assert!(!script.contains("settings put global global_http_proxy_host"));
        assert!(!script.contains("settings put global global_http_proxy_port"));
        assert!(!script.contains("settings put global global_proxy_pac_url"));
        let delete_at = script.find("settings delete global global_http_proxy_port").unwrap();
        let put_at = script.find("settings put global http_proxy :0").unwrap();
        assert!(delete_at < put_at, "必须先删再写 :0，否则拆出的键会残留");
    }

    #[test]
    fn a_forward_port_conflict_is_retried_but_other_failures_are_not() {
        // The message shapes `adb forward` actually prints when the local port is taken.
        assert!(looks_like_port_conflict(
            "adb: error: cannot bind listener: Address already in use"
        ));
        assert!(looks_like_port_conflict(
            "cannot bind to 127.0.0.1:19333: Address already in use"
        ));
        // A dead socket or an offline device must surface as the original error.
        assert!(!looks_like_port_conflict("error: device offline"));
        assert!(!looks_like_port_conflict("error: closed"));
    }

    #[test]
    fn a_fallback_forward_port_is_free_and_differs_from_the_taken_one() {
        let held = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let taken = held.local_addr().unwrap().port();
        let alt = pick_forward_port(taken).expect("应能找到一个空闲端口");
        assert_ne!(alt, taken, "备用端口不能是那个已被占用的端口");
        assert!(alt > 0, "0 对 adb 意味着“任意端口”，不能作为具体端口下发");
        // The number must actually be bindable right now.
        TcpListener::bind(("127.0.0.1", alt)).expect("备用端口应可用");
        drop(held);
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
