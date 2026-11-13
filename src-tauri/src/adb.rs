//! `adb` discovery, device listing and logcat command construction.
//!
//! The app shells out to the platform `adb` binary instead of linking a USB library, so
//! no extra native dependency is needed and users keep whatever adb / device setup they
//! already have. Notes:
//!
//! - Discovery order matters: an explicit `ADB_PATH` wins, then the Android SDK
//!   variables, then `PATH`, then the usual install locations. Users who installed
//!   Platform-Tools but never touched their shell environment still get a hit.
//! - Every short-lived call is bounded by [`ADB_TIMEOUT`]. `adb devices` can block for
//!   seconds while it starts the local server, and a wedged server must not freeze the UI.
//! - On Windows the child is created with `CREATE_NO_WINDOW`, otherwise every command
//!   flashes a console window on top of the app.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ADB_TIMEOUT: Duration = Duration::from_secs(10);

/// Reasons are written for the person in front of the screen, not for a log file.
pub const NOT_FOUND: &str = "未找到 adb。请安装 Android SDK Platform-Tools，\
并确保 adb 在 PATH 中，或设置 ANDROID_HOME / ANDROID_SDK_ROOT / ADB_PATH 指向它。";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AdbStatus {
    pub available: bool,
    pub path: String,
    /// First line of `adb version`, e.g. `Android Debug Bridge version 1.0.41`.
    pub version: String,
    /// Non-empty when adb is missing or `adb version` failed.
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AdbDevice {
    pub serial: String,
    /// `device` (ready) | `unauthorized` | `offline` | `no permissions` | ...
    pub state: String,
    pub model: String,
    pub product: String,
    pub device: String,
    pub transport_id: String,
}

impl AdbDevice {
    pub fn is_ready(&self) -> bool {
        self.state == "device"
    }

    /// `SM_G9730 (R58M12ABCDE)` — what the device picker shows.
    pub fn label(&self) -> String {
        let name = if self.model.is_empty() { &self.serial } else { &self.model };
        format!("{name} ({})", self.serial)
    }
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Absolute paths that cover the default SDK install per platform.
fn common_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            out.push(Path::new(&home).join("Library/Android/sdk/platform-tools").join(exe("adb")));
        }
        out.push(PathBuf::from("/opt/homebrew/bin").join(exe("adb")));
        out.push(PathBuf::from("/usr/local/bin").join(exe("adb")));
    } else if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(Path::new(&local).join("Android/Sdk/platform-tools").join(exe("adb")));
        }
        out.push(PathBuf::from(r"C:\Android\platform-tools").join(exe("adb")));
    } else {
        if let Some(home) = std::env::var_os("HOME") {
            out.push(Path::new(&home).join("Android/Sdk/platform-tools").join(exe("adb")));
        }
        out.push(PathBuf::from("/usr/bin").join(exe("adb")));
    }
    out
}

pub fn locate() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("ADB_PATH") {
        let path = PathBuf::from(explicit.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    for var in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Ok(root) = std::env::var(var) {
            let path = Path::new(root.trim()).join("platform-tools").join(exe("adb"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let path = dir.join(exe("adb"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    common_paths().into_iter().find(|path| path.is_file())
}

/// A `Command` for adb with the platform quirks already applied.
pub fn command(adb: &Path) -> Command {
    let mut cmd = Command::new(adb);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null());
    cmd
}

/// Run a short adb subcommand and return stdout. Bounded by [`ADB_TIMEOUT`].
///
/// Only use this for commands with small output (`version`, `devices`, `logcat -c`).
/// Anything that can produce a full log buffer must be streamed (see `stream.rs`) — piping
/// it here without draining while waiting would deadlock.
pub(crate) fn run(adb: &Path, args: &[String]) -> Result<String, String> {
    let mut cmd = command(adb);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 adb: {e}"))?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if started.elapsed() > ADB_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "adb {} 超时（{} 秒无响应）",
                        args.join(" "),
                        ADB_TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("等待 adb 结束失败: {e}")),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("读取 adb 输出失败: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let text = stderr.trim();
        if !text.is_empty() {
            return Err(text.to_string());
        }
        return Err(format!("adb {} 失败（退出码 {:?}）", args.join(" "), output.status.code()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn status() -> AdbStatus {
    let Some(adb) = locate() else {
        return AdbStatus {
            available: false,
            error: NOT_FOUND.to_string(),
            ..Default::default()
        };
    };
    match run(&adb, &["version".to_string()]) {
        Ok(text) => AdbStatus {
            available: true,
            path: adb.display().to_string(),
            version: text.lines().next().unwrap_or("").trim().to_string(),
            error: String::new(),
        },
        Err(e) => AdbStatus {
            available: false,
            path: adb.display().to_string(),
            version: String::new(),
            error: e,
        },
    }
}

pub fn devices() -> Result<Vec<AdbDevice>, String> {
    let adb = locate().ok_or_else(|| NOT_FOUND.to_string())?;
    let text = run(&adb, &["devices".to_string(), "-l".to_string()])?;
    Ok(parse_devices(&text))
}

/// Parse `adb devices -l`. Kept pure so the format quirks are unit-testable:
///
/// ```text
/// List of devices attached
/// R58M12ABCDE   device product:beyond1q model:SM_G9730 device:beyond1q transport_id:3
/// 0123456789AB  unauthorized usb:1-1 transport_id:2
/// 9A1B2C3D      no permissions (user in plugdev group; are your udev rules wrong?)
/// ```
pub fn parse_devices(text: &str) -> Vec<AdbDevice> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with("List of devices")
            || line.starts_with("adb server")
            || line.starts_with('*')
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(serial) = parts.next() else { continue };
        let Some(state) = parts.next() else { continue };
        let rest: Vec<&str> = parts.collect();

        let mut device = AdbDevice {
            serial: serial.to_string(),
            state: state.to_string(),
            ..Default::default()
        };
        // `no permissions (...)` is split into two tokens by whitespace.
        if state == "no" && rest.first().map(|t| t.starts_with("permissions")).unwrap_or(false) {
            device.state = "no permissions".to_string();
        }
        for token in rest {
            let Some((key, value)) = token.split_once(':') else { continue };
            match key {
                "model" => device.model = value.to_string(),
                "product" => device.product = value.to_string(),
                "device" => device.device = value.to_string(),
                "transport_id" => device.transport_id = value.to_string(),
                _ => {}
            }
        }
        out.push(device);
    }
    out
}

/// `adb -s <serial> logcat ...` argument list for a live capture.
///
/// `-v threadtime` is mandatory: it is the only format carrying pid/tid/tag/level in a
/// shape the parser understands. No `-d`, no `-t` — the process stays attached and a plain
/// `adb logcat` already replays the existing ring buffer before following new lines, so the
/// live capture covers the "what already happened" case too.
pub fn logcat_args(serial: &str, buffers: &[String]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-s".into(),
        serial.to_string(),
        "logcat".into(),
        "-v".into(),
        "threadtime".into(),
    ];
    for buffer in buffers {
        let buffer = buffer.trim();
        if buffer.is_empty() {
            continue;
        }
        args.push("-b".into());
        args.push(buffer.to_string());
    }
    args
}

pub fn clear_args(serial: &str, buffers: &[String]) -> Vec<String> {
    let mut args: Vec<String> = vec!["-s".into(), serial.to_string(), "logcat".into(), "-c".into()];
    for buffer in buffers {
        let buffer = buffer.trim();
        if buffer.is_empty() {
            continue;
        }
        args.push("-b".into());
        args.push(buffer.to_string());
    }
    args
}

pub fn clear_logcat(adb: &Path, serial: &str, buffers: &[String]) -> Result<(), String> {
    run(adb, &clear_args(serial, buffers)).map(|_| ())
}

/// `adb -s <serial> shell ps -A -o PID,NAME` — one row per running process.
///
/// This is how a package is recovered for log lines: `-v threadtime` has no package column,
/// but an app process is named after its package. Measured at ~120ms for ~480 rows on a
/// real phone, so a live capture can afford to refresh it every few seconds.
pub fn ps_args(serial: &str) -> Vec<String> {
    vec![
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
        "ps".to_string(),
        "-A".to_string(),
        "-o".to_string(),
        "PID,NAME".to_string(),
    ]
}

/// Best-effort process snapshot. An empty string means "no mapping this round" — a missing
/// package is a degraded label, never a reason to fail the capture.
pub fn process_snapshot(adb: &Path, serial: &str) -> String {
    run(adb, &ps_args(serial)).unwrap_or_default()
}

/// `adb -s <serial> shell "<script>"` — one device round trip per probe question.
pub fn shell_args(serial: &str, script: &str) -> Vec<String> {
    vec![
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
        script.to_string(),
    ]
}

/// Run a read-only probe script. Failure is *data*, not an error: an absent `su`, a
/// non-debuggable package or a missing property all say something about which capture
/// routes are open. The wrapper swallows stderr and forces a zero exit status so
/// [`run`] returns stdout instead of turning it into an error string, while still
/// keeping its 10s timeout so a wedged adb server cannot freeze the UI.
pub fn shell_probe(adb: &Path, serial: &str, script: &str) -> String {
    let wrapped = format!("{{ {script} ; }} 2>/dev/null; true");
    run(adb, &shell_args(serial, &wrapped)).unwrap_or_default()
}

/// `adb -s <serial> forward tcp:<port> localabstract:<socket>`.
pub fn forward_args(serial: &str, port: u16, socket: &str) -> Vec<String> {
    vec![
        "-s".to_string(),
        serial.to_string(),
        "forward".to_string(),
        format!("tcp:{port}"),
        format!("localabstract:{socket}"),
    ]
}

/// `adb -s <serial> forward --remove tcp:<port>` — always paired with a successful
/// forward, otherwise the PC port stays occupied after the capture ends.
pub fn forward_remove_args(serial: &str, port: u16) -> Vec<String> {
    vec![
        "-s".to_string(),
        serial.to_string(),
        "forward".to_string(),
        "--remove".to_string(),
        format!("tcp:{port}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_ready_unauthorized_and_offline_devices() {
        let text = "List of devices attached\n\
                    R58M12ABCDE\tdevice product:beyond1q model:SM_G9730 device:beyond1q transport_id:3\n\
                    0123456789AB\tunauthorized usb:1-1 transport_id:2\n\
                    emulator-5554\toffline transport_id:1\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].serial, "R58M12ABCDE");
        assert_eq!(devices[0].state, "device");
        assert_eq!(devices[0].model, "SM_G9730");
        assert_eq!(devices[0].transport_id, "3");
        assert!(devices[0].is_ready());
        assert_eq!(devices[1].state, "unauthorized");
        assert!(!devices[1].is_ready());
        assert_eq!(devices[2].state, "offline");
        assert!(devices[2].model.is_empty());
    }

    #[test]
    fn collapses_two_token_no_permissions_state() {
        let text = "List of devices attached\n\
                    9A1B2C3D\tno permissions (user in plugdev group; are your udev rules wrong?)\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].state, "no permissions");
    }

    #[test]
    fn ignores_empty_output() {
        assert!(parse_devices("List of devices attached\n\n").is_empty());
    }

    #[test]
    fn builds_live_threadtime_and_clear_args() {
        let stream = logcat_args("S1", &["main".into(), "crash".into()]);
        assert_eq!(
            stream,
            vec!["-s", "S1", "logcat", "-v", "threadtime", "-b", "main", "-b", "crash"]
        );
        // A live capture must never carry `-d`/`-t`: those would exit instead of following.
        assert!(!logcat_args("S1", &[]).iter().any(|a| a == "-d" || a == "-t"));
        let clear = clear_args("S1", &["system".into()]);
        assert_eq!(clear, vec!["-s", "S1", "logcat", "-c", "-b", "system"]);
    }

    #[test]
    fn builds_the_process_snapshot_args() {
        // `ps -A -o PID,NAME` is what makes `pid → package` possible: the parser relies on
        // the one-row-per-process shape it produces.
        assert_eq!(
            ps_args("S1"),
            vec!["-s", "S1", "shell", "ps", "-A", "-o", "PID,NAME"]
        );
    }

    #[test]
    fn device_label_falls_back_to_serial() {
        let mut device = AdbDevice { serial: "ABC".into(), ..Default::default() };
        assert_eq!(device.label(), "ABC (ABC)");
        device.model = "Pixel_7".into();
        assert_eq!(device.label(), "Pixel_7 (ABC)");
    }

    // The two tests below touch a real phone, so they stay opt-in:
    //   cd src-tauri && cargo test --lib -- --ignored
    // They are the only way to prove that discovery, `adb devices -l` parsing and a real
    // `logcat -d` dump line up with the parser on an actual device.

    #[test]
    #[ignore = "需要本机安装 adb 并连接设备"]
    fn locates_adb_and_lists_real_devices() {
        let status = status();
        assert!(status.available, "adb 未就绪: {}", status.error);
        assert!(status.version.contains("Android Debug Bridge"), "{}", status.version);
        assert!(!status.path.is_empty());

        let devices = devices().expect("adb devices 应成功");
        assert!(!devices.is_empty(), "没有检测到设备，先连接一台再跑");
        assert!(devices.iter().any(AdbDevice::is_ready), "没有已授权的设备");
    }

    #[test]
    #[ignore = "需要本机安装 adb 并连接设备"]
    fn real_device_threadtime_output_parses() {
        let adb_path = locate().expect("应找到 adb");
        let serial = devices()
            .expect("adb devices 应成功")
            .into_iter()
            .find(AdbDevice::is_ready)
            .expect("需要一台已授权设备")
            .serial;

        // One-shot `-d` lives only here: it is the cheapest way to grab real threadtime
        // output for the parser. The app itself only streams.
        let temp = std::env::temp_dir().join("loglens-adb-test.log");
        let file = fs::File::create(&temp).expect("应能创建临时文件");
        let status = command(&adb_path)
            .args(["-s", &serial, "logcat", "-v", "threadtime", "-d", "-t", "300"])
            .stdout(Stdio::from(file))
            .stderr(Stdio::piped())
            .output()
            .expect("adb logcat -d 应能执行");
        assert!(status.status.success(), "adb logcat -d 失败");
        let bytes = fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
        assert!(bytes > 0, "adb logcat -d 输出为空");

        let parsed = crate::logcat::LogcatParser::parse_file(&temp).expect("解析应成功");
        let _ = fs::remove_file(&temp);
        assert!(parsed.entries.len() > 1, "应解析出多条日志: {parsed:?}");
        // `-v threadtime` always carries a level, so nearly every line must be recognised.
        let unknown = parsed
            .entries
            .iter()
            .filter(|e| e.level == "?")
            .count();
        assert!(
            unknown * 4 < parsed.entries.len(),
            "未识别行过多（{unknown}/{}），threadtime 解析可能失配",
            parsed.entries.len()
        );
    }
}
