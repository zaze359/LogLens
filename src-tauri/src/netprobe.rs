//! "Which capture routes are open on this device?" — a read-only capability probe.
//!
//! The answer to "can I see HTTPS plaintext without touching the app" is decided by the
//! device, not by the tool. Three facts settle it:
//!
//!   1. a `webview_devtools_remote_<pid>` socket means CDP can read WebView HTTPS
//!      plaintext with no root and no certificate (Android lets a *debuggable* app keep
//!      WebView debugging on even in a `user` build);
//!   2. a root shell means a MITM CA can be promoted into the system trust store, which
//!      unlocks plaintext for *all* apps including native OkHttp;
//!   3. a debuggable target package helps only if its APK declares `debug-overrides`
//!      trusting user CAs — `debuggable` alone does **not** make Android 7+ trust them.
//!
//! Every probe here is a read-only `adb shell` command. The one stateful step, an
//! `adb forward` used to query `/json/version`, is removed immediately afterwards.

use crate::{adb, cdp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Port used for the short-lived `/json/version` probe. A capture may pick another one.
pub const PROBE_PORT: u16 = 19222;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PackageProbe {
    pub package: String,
    pub debuggable: bool,
    pub target_sdk: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DevtoolsProbe {
    pub pid: u32,
    pub socket: String,
    pub package: String,
    pub browser: String,
    /// Live page targets. Zero is not "unavailable": the app simply has no H5 page open.
    pub pages: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceEvidence {
    pub serial: String,
    pub android_release: String,
    pub sdk: u32,
    /// `user` | `userdebug` | `eng`
    pub build_type: String,
    pub debuggable_build: bool,
    pub root_shell: bool,
    /// Inferred from `ro.secure`: `adb root` is refused on production builds.
    pub adb_root_likely: bool,
    pub proxy: String,
    pub packages: Vec<PackageProbe>,
    pub devtools: Vec<DevtoolsProbe>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptureRoute {
    /// `cdp` | `mitm-root` | `mitm-debug` | `proxy-metadata`
    pub id: String,
    pub label: String,
    pub https_plaintext: bool,
    pub coverage: String,
    pub available: bool,
    pub reason: String,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProbeReport {
    pub evidence: DeviceEvidence,
    pub routes: Vec<CaptureRoute>,
    pub recommended: String,
    pub summary: String,
    /// Non-empty when this probe automatically undid a proxy left by an earlier run.
    #[serde(default)]
    pub healed_proxy: String,
}

fn props_script() -> &'static str {
    "echo REL=$(getprop ro.build.version.release); echo SDK=$(getprop ro.build.version.sdk); \
     echo TYPE=$(getprop ro.build.type); echo DBG=$(getprop ro.debuggable); \
     echo SEC=$(getprop ro.secure)"
}

/// 探针输出里分隔「键值对」和「socket 列表」两段的标记。dumpsys / getprop 都不会产出它。
///
/// 不能用 `#` 开头：`adb::shell_probe` 把脚本包成单行的 `{ … ; }`，`#` 会把行尾的
/// `; }` 一起注释掉，整条 shell 因为花括号不闭合而失败（输出全空，还看不到报错）。
const UNIX_MARK: &str = "__UNIX__";

/// 一次 `dumpsys` 覆盖全部已安装包。
///
/// 之前是「每个包跑一次 `dumpsys package <pkg>`」，24 个包串行 ≈ 6.5s（单次实测 270ms），
/// 而 `dumpsys package packages` 全量 191 个包只花 0.45s。grep 在设备端执行，回传的是
/// 每包两三行（约 30KB），所以传输量也不是问题。
const PACKAGE_FLAGS_SCRIPT: &str =
    "dumpsys package packages | grep -E '^  Package \\[|^    flags=\\[|targetSdk='";

/// props + 代理设置 + WebView 调试 socket，一次 shell 拿完。
///
/// 拆成三次 `adb shell` 意味着三次 USB 往返（每次 150–250ms），而 `settings get` 只有五个
/// 键，不值得单独跑一趟。
fn evidence_script() -> String {
    format!(
        "{}; {}; echo {UNIX_MARK}; cat /proc/net/unix | grep -i devtools",
        props_script(),
        proxy_settings_script()
    )
}

/// 按 [`UNIX_MARK`] 把探针输出切成「键值对段」和「socket 段」。
fn split_evidence(text: &str) -> (&str, &str) {
    match text.split_once(UNIX_MARK) {
        Some((labeled, unix)) => (labeled, unix),
        None => (text, ""),
    }
}

/// 解析 `dumpsys package packages` 的过滤结果。
///
/// 输出是「包段落」串起来的：每个包以 `  Package [name] (...)` 开头，段内有
/// `    versionCode=… minSdk=… targetSdk=…` 和 `    flags=[ DEBUGGABLE … ]`。
/// 注意权限行里也会出现 `flags=[`，但不在行首，按行首缩进匹配即可排除。
pub fn parse_package_flags(text: &str) -> Vec<PackageProbe> {
    let mut out: Vec<PackageProbe> = Vec::new();
    let mut current: Option<PackageProbe> = None;

    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("Package [") {
            if let Some(done) = current.take() {
                out.push(done);
            }
            let name = rest.split(']').next().unwrap_or("").trim();
            if !name.is_empty() {
                current = Some(PackageProbe {
                    package: name.to_string(),
                    debuggable: false,
                    target_sdk: 0,
                });
            }
            continue;
        }
        let Some(probe) = current.as_mut() else {
            continue;
        };
        if trimmed.starts_with("flags=[") && trimmed.contains("DEBUGGABLE") {
            probe.debuggable = true;
        }
        if let Some(pos) = trimmed.find("targetSdk=") {
            let digits: String = trimmed[pos + "targetSdk=".len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(sdk) = digits.parse::<u32>() {
                probe.target_sdk = sdk;
            }
        }
    }
    if let Some(done) = current.take() {
        out.push(done);
    }
    out
}

pub fn parse_labeled(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

pub fn parse_package_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// `dumpsys package <pkg>` carries both the flags line and the SDK line. On Android 15
/// they appear as `flags=[ DEBUGGABLE HAS_CODE ... ]` and `... targetSdk=36 ...`.
pub fn parse_dumpsys(text: &str) -> (bool, u32) {
    let mut debuggable = false;
    let mut target_sdk = 0u32;
    for line in text.lines() {
        if line.contains("flags=[") && line.contains("DEBUGGABLE") {
            debuggable = true;
        }
        if let Some(pos) = line.find("targetSdk=") {
            let digits: String = line[pos + "targetSdk=".len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(sdk) = digits.parse::<u32>() {
                target_sdk = sdk;
            }
        }
    }
    (debuggable, target_sdk)
}

pub fn parse_cmdline(text: &str) -> String {
    let first = text
        .split(|c: char| c.is_whitespace() || c == '\0')
        .find(|part| !part.is_empty())
        .unwrap_or("");
    if first.is_empty() {
        return String::new();
    }
    // Remote processes are named `<package>:<service>`; the CDP socket belongs to the app.
    first.split(':').next().unwrap_or(first).to_string()
}

/// `cat /proc/net/unix` rows look like
/// `... 0001 01 381180 @webview_devtools_remote_13311`.
pub fn parse_devtools_sockets(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        // The abstract socket name is the last column, prefixed with `@`.
        let Some(token) = line.split_whitespace().last() else {
            continue;
        };
        if !token.contains("devtools_remote_") {
            continue;
        }
        let name = token.trim_start_matches('@');
        let digits: String = name
            .rsplit('_')
            .next()
            .unwrap_or("")
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(pid) = digits.parse::<u32>() {
            if pid > 0 {
                out.push((pid, name.to_string()));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub fn parse_proxy(text: &str) -> String {
    let value = text.trim();
    // `:0` is Android's standard "no proxy" value. We write it explicitly when
    // restoring (see restore_script), so it must read back as "no proxy" too.
    if value.is_empty() || value == "null" || value == ":0" || value == "0.0.0.0:0" {
        String::new()
    } else {
        value.to_string()
    }
}

/// AOSP splits `settings put global http_proxy <host>:<port>` into these keys, and
/// `settings delete global http_proxy` does **not** remove the split ones — verified on
/// an Android 15 tablet, where the device kept a dead proxy after the capture ended.
/// Any code that sets or restores a proxy must handle the whole set.
pub const PROXY_KEYS: &[&str] = &[
    "http_proxy",
    "global_http_proxy_host",
    "global_http_proxy_port",
    "global_http_proxy_exclusion_list",
    "global_proxy_pac_url",
];

pub fn proxy_settings_script() -> String {
    PROXY_KEYS
        .iter()
        .map(|key| format!("echo {key}=$(settings get global {key})"))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn parse_proxy_settings(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .map(|(key, value)| (key.trim().to_string(), parse_proxy(value)))
        .collect()
}

/// Human-readable current proxy, whether stored as `http_proxy` or as split host/port.
pub fn proxy_from_settings(text: &str) -> String {
    let values = parse_proxy_settings(text);
    if let Some(direct) = values.get("http_proxy") {
        if !direct.is_empty() {
            return direct.clone();
        }
    }
    let host = values.get("global_http_proxy_host").cloned().unwrap_or_default();
    if host.is_empty() {
        return String::new();
    }
    let port = values.get("global_http_proxy_port").cloned().unwrap_or_default();
    if port.is_empty() {
        host
    } else {
        format!("{host}:{port}")
    }
}

pub fn list_packages(adb_path: &Path, serial: &str) -> Vec<String> {
    parse_package_list(&adb::shell_probe(adb_path, serial, "pm list packages -3"))
}

/// Assess without touching a device, so the decision table is unit-testable.
pub fn assess(evidence: &DeviceEvidence) -> (Vec<CaptureRoute>, String, String) {
    let mut routes = Vec::new();

    let cdp_socket = evidence.devtools.first();
    let cdp_open = cdp_socket.is_some();
    let cdp_pages: u32 = evidence.devtools.iter().map(|d| d.pages).sum();
    routes.push(CaptureRoute {
        id: "cdp".into(),
        label: "WebView CDP：零改动看 HTTPS 明文".into(),
        https_plaintext: true,
        coverage: "WebView 内的 H5 / 混合页面请求".into(),
        available: cdp_open,
        reason: if cdp_open {
            let s = cdp_socket.unwrap();
            if cdp_pages > 0 {
                format!(
                    "检测到 {}（pid {}）的 WebView 调试通道，当前有 {} 个页面，可直接读取 HTTPS 明文。",
                    s.package, s.pid, cdp_pages
                )
            } else {
                format!(
                    "检测到 {}（pid {}）的 WebView 调试通道；当前没有打开的页面，在 App 里进入任一 H5 页面后即可采集。",
                    s.package, s.pid
                )
            }
        } else {
            "设备上没有 webview_devtools_remote_* socket（App 未开启 WebView 调试，或当前没有 WebView 进程）。".into()
        },
        steps: vec![
            "adb forward tcp:<端口> localabstract:webview_devtools_remote_<pid>".into(),
            "连接 /json/list 里的 webSocketDebuggerUrl，发送 Network.enable".into(),
            "在 App 里打开目标 H5 页面，请求会实时进入采集列表".into(),
        ],
    });

    let root_open = evidence.root_shell || evidence.adb_root_likely;
    routes.push(CaptureRoute {
        id: "mitm-root".into(),
        label: "MITM + 系统证书：全设备 HTTPS 明文".into(),
        https_plaintext: true,
        coverage: "全设备 HTTP(S)，含原生 OkHttp 请求".into(),
        available: root_open,
        reason: if root_open {
            "设备有 root / 非生产构建，可把 MITM CA 放进系统信任区，覆盖所有 App。".into()
        } else {
            format!(
                "Android {}（{} 构建）没有 root：su 不可用，adb root 在 production build 上会被拒绝，系统 CA 目录不可写。",
                evidence.android_release, evidence.build_type
            )
        },
        steps: vec![
            "把 MITM 根证书按 subject_hash_old 命名后写入 /system/etc/security/cacerts/（Android 14+ 还需处理 APEX conscrypt 挂载）".into(),
            "设置全局代理指向本机 mitmdump，启动抓包".into(),
            "结束时删除代理设置，恢复设备网络".into(),
        ],
    });

    let debug_pkg = evidence.packages.iter().find(|p| p.debuggable);
    routes.push(CaptureRoute {
        id: "mitm-debug".into(),
        label: "MITM + 用户证书：需 APK 已声明 debug-overrides".into(),
        https_plaintext: false,
        coverage: "取决于 APK 的网络安全配置".into(),
        available: debug_pkg.is_some(),
        reason: if let Some(p) = debug_pkg {
            format!(
                "{} 是 DEBUGGABLE（targetSdk {}），但 Android 7+ 的 App 默认仍不信任用户 CA；只有 APK 的 networkSecurityConfig 里声明了 <debug-overrides> 放行 user 证书时才能看到明文。",
                p.package, p.target_sdk
            )
        } else {
            "所选包都不是 debuggable，用户安装的 CA 证书对这些 App 完全不生效。".into()
        },
        steps: vec![
            "先实测：装用户 CA + 设代理，看目标 App 是否报证书错误".into(),
            "若报 SSLHandshakeException，说明 APK 未放行，只能退回 CDP 或元数据路线".into(),
            "有证书固定（CertificatePinner）时即使放行也不会成功".into(),
        ],
    });

    routes.push(CaptureRoute {
        id: "proxy-metadata".into(),
        label: "代理元数据：明看 HTTP，暗看域名与时序".into(),
        https_plaintext: false,
        coverage: "明文 HTTP + HTTPS 的 CONNECT 域名、连接耗时、失败原因".into(),
        available: true,
        reason: "不需要任何信任配置，任何时候都可用；HTTPS 只能看到目标域名，看不到 URL 路径与内容。".into(),
        steps: vec![
            "设置全局代理指向本机 mitmdump".into(),
            "HTTPS 请求以 CONNECT host:443 形式出现，可统计域名、耗时与失败".into(),
            "或改用 VpnService 型工具导出 HAR / PCAP 拿 SNI、DNS 与连接时序".into(),
        ],
    });

    // A deterministic route beats a conditional one. `mitm-debug` is "maybe, after a
    // certificate test", so it is offered as an option but never as the recommendation —
    // otherwise the UI would tell the user to do work that may be impossible anyway.
    let recommended = routes
        .iter()
        .find(|r| r.available && r.https_plaintext)
        .or_else(|| {
            routes
                .iter()
                .find(|r| r.id == "proxy-metadata" && r.available)
        })
        .map(|r| r.id.clone())
        .unwrap_or_else(|| "proxy-metadata".into());

    let summary = match recommended.as_str() {
        "cdp" => "这台设备可以不改 App、不 root 就拿到 HTTPS 明文：走 WebView CDP。注意只覆盖 WebView 发出的请求，原生接口仍需代理路线。".to_string(),
        "mitm-root" => "设备可 root，走 MITM + 系统证书可以拿到全 App 的 HTTPS 明文。".to_string(),
        "mitm-debug" => "目标包是 debug 包，先实测用户 CA 是否被信任；不放行就退回 CDP / 元数据路线。".to_string(),
        _ => "在当前设备上拿不到 HTTPS 明文：只能看明文 HTTP、HTTPS 域名与连接时序。要明文需要 WebView 调试通道或系统证书权限。".to_string(),
    };
    (routes, recommended, summary)
}

/// Run every read-only probe against a connected device and grade the capture routes.
pub fn probe(adb_path: &Path, serial: &str, packages: &[String]) -> Result<ProbeReport, String> {
    // 一次 shell 拿 props / 代理 / socket；`packages` 只在下面那条慢路径上用到。
    let evidence_text = adb::shell_probe(adb_path, serial, &evidence_script());
    let (labeled, unix) = split_evidence(&evidence_text);
    let props = parse_labeled(labeled);
    let proxy = proxy_from_settings(labeled);

    // `command -v su` 先短路：没有 su 的设备上跑 `su -c id` 是白等一趟 shell（有些机型还会
    // 卡到超时），而 su 存在与否本来就是这里唯一要判断的事。
    let has_su = !adb::shell_probe(adb_path, serial, "command -v su").trim().is_empty();
    let root = if has_su {
        adb::shell_probe(adb_path, serial, "su -c id")
    } else {
        String::new()
    };

    let mut probed = parse_package_flags(&adb::shell_probe(adb_path, serial, PACKAGE_FLAGS_SCRIPT));
    if probed.is_empty() {
        // 极少数定制 ROM 的 dumpsys 缩进不一样：退回逐个探测。限 24 个包，够判断有没有
        // DEBUGGABLE 的包（`mitm-debug` 路线只关心「有没有」）。
        let list: Vec<String> = if packages.is_empty() {
            list_packages(adb_path, serial)
        } else {
            packages.to_vec()
        };
        for package in list.iter().take(24) {
            let script = format!("dumpsys package {package} | grep -E 'targetSdk=|flags=\\['");
            let (debuggable, target_sdk) = parse_dumpsys(&adb::shell_probe(adb_path, serial, &script));
            probed.push(PackageProbe {
                package: package.clone(),
                debuggable,
                target_sdk,
            });
        }
    }

    let mut devtools = Vec::new();
    for (pid, socket) in parse_devtools_sockets(unix) {
        let cmdline = adb::shell_probe(
            adb_path,
            serial,
            &format!("cat /proc/{pid}/cmdline | tr '\\0' ' '"),
        );
        let package = parse_cmdline(&cmdline);
        let mut browser = String::new();
        let mut pages = 0u32;
        if adb::run(adb_path, &adb::forward_args(serial, PROBE_PORT, &socket)).is_ok() {
            browser = cdp::version(PROBE_PORT).unwrap_or_default().browser;
            pages = cdp::targets(PROBE_PORT)
                .unwrap_or_default()
                .iter()
                .filter(|t| t.kind == "page")
                .count() as u32;
            // The forward is ours; remove it even though the port is reused by captures.
            let _ = adb::run(adb_path, &adb::forward_remove_args(serial, PROBE_PORT));
        }
        devtools.push(DevtoolsProbe {
            pid,
            socket,
            package,
            browser,
            pages,
        });
    }

    let secure = props.get("SEC").map(|s| s.as_str()).unwrap_or("1");
    let evidence = DeviceEvidence {
        serial: serial.to_string(),
        android_release: props.get("REL").cloned().unwrap_or_default(),
        sdk: props.get("SDK").and_then(|s| s.parse().ok()).unwrap_or(0),
        build_type: props.get("TYPE").cloned().unwrap_or_default(),
        debuggable_build: props.get("DBG").map(|s| s == "1").unwrap_or(false),
        root_shell: root.contains("uid=0"),
        adb_root_likely: secure == "0",
        proxy,
        packages: probed,
        devtools,
    };
    let (routes, recommended, summary) = assess(&evidence);
    Ok(ProbeReport {
        evidence,
        routes,
        recommended,
        summary,
        healed_proxy: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_split_proxy_keys_android_actually_writes() {
        let text = "http_proxy=null\n\
                    global_http_proxy_host=172.21.33.28\n\
                    global_http_proxy_port=18999\n\
                    global_http_proxy_exclusion_list=null\n\
                    global_proxy_pac_url=null\n";
        let values = parse_proxy_settings(text);
        assert_eq!(values["global_http_proxy_host"], "172.21.33.28");
        assert_eq!(values["http_proxy"], "");
        assert_eq!(proxy_from_settings(text), "172.21.33.28:18999");
        assert!(proxy_settings_script().contains("global_http_proxy_port"));
        assert_eq!(
            proxy_from_settings("http_proxy=null\nglobal_http_proxy_host=null"),
            ""
        );
    }

    #[test]
    fn splits_the_single_shell_evidence_into_props_and_sockets() {
        // `evidence_script()` 一次 shell 输出的形状：键值对 → 标记 → /proc/net/unix 行。
        let text = "REL=15\nSDK=35\nTYPE=user\nDBG=0\nSEC=1\nhttp_proxy=null\n\
                    __UNIX__\n\
                    00000000: 00000002 00000000 00010000 0001 01 23456 @webview_devtools_remote_13311\n";
        let (labeled, unix) = split_evidence(text);
        let props = parse_labeled(labeled);
        assert_eq!(props["SDK"], "35");
        assert_eq!(props["TYPE"], "user");
        // 标记行不能被当成键值对混进 props。
        assert!(!props.contains_key(UNIX_MARK));

        let sockets = parse_devtools_sockets(unix);
        assert_eq!(sockets.len(), 1);
        assert_eq!(sockets[0].0, 13311);
        assert!(sockets[0].1.contains("webview_devtools_remote"));
    }

    /// 合成脚本会被包进单行 `{ … ; }`，任何 `#` 都会把 `; }` 注释掉并让整条 shell 失败。
    #[test]
    fn evidence_script_is_safe_inside_a_single_line_shell_block() {
        let script = evidence_script();
        assert!(!script.contains('#'), "脚本里不能出现 #：{script}");
        assert!(script.contains(UNIX_MARK));
        assert!(script.contains("getprop ro.build.version.release"));
        assert!(script.contains("settings get global"));
    }

    /// 批量 `dumpsys package packages` 的解析：一次覆盖全部包，取代「每包一次 dumpsys」。
    #[test]
    fn parses_package_flags_from_the_bulk_dumpsys() {
        // 真实形状（Android 15）：targetSdk 与 versionCode 同行；权限行里也有 `flags=[`，
        // 但它不在行首，不能被误当成包级 flags。
        let text = "\
  Package [com.example.app] (1a2b3c):
    versionCode=35 minSdk=24 targetSdk=35
    flags=[ DEBUGGABLE HAS_CODE ALLOW_BACKUP ]
      android.permission.CAMERA: granted=true, flags=[ USER_SENSITIVE_WHEN_GRANTED]
  Package [com.vendor.locked] (9f8e7d):
    versionCode=1 minSdk=29 targetSdk=34
    flags=[ SYSTEM HAS_CODE ]
";
        let packages = parse_package_flags(text);
        assert_eq!(packages.len(), 2, "两个包段落都应解析出来");
        assert!(packages[0].debuggable, "DEBUGGABLE 应被识别");
        assert_eq!(packages[0].target_sdk, 35);
        assert!(!packages[1].debuggable);
        assert_eq!(packages[1].target_sdk, 34);

        // 格式不认识时返回空，`probe()` 据此退回逐个探测。
        assert!(parse_package_flags("").is_empty());
        assert!(parse_package_flags("dumpsys: not found").is_empty());
    }

    #[test]
    fn parses_props_packages_and_dumpsys() {
        let props = parse_labeled("REL=15\nSDK=35\nTYPE=user\nDBG=0\nSEC=1\n");
        assert_eq!(props["REL"], "15");
        assert_eq!(props["SDK"], "35");
        assert_eq!(parse_package_list("package:com.a\npackage:com.b\njunk\n").len(), 2);
        let (debug, sdk) = parse_dumpsys(
            "    flags=[ DEBUGGABLE HAS_CODE ALLOW_BACKUP ]\n    versionCode=999999 minSdk=29 targetSdk=36\n",
        );
        assert!(debug);
        assert_eq!(sdk, 36);
        assert_eq!(parse_dumpsys("flags=[ HAS_CODE ]\n").0, false);
    }

    #[test]
    fn extracts_devtools_socket_and_owner_package() {
        let row = "0000000000000000: 00000002 00000000 00010000 0001 01 381180 @webview_devtools_remote_13311";
        assert_eq!(parse_devtools_sockets(row), vec![(13311, "webview_devtools_remote_13311".to_string())]);
        assert_eq!(parse_cmdline("com.hailiang.workcloud.teacher\0\0"), "com.hailiang.workcloud.teacher");
        assert_eq!(parse_cmdline("com.x:remote\0"), "com.x");
        assert_eq!(parse_proxy("null"), "");
        assert_eq!(parse_proxy("10.0.0.2:8080"), "10.0.0.2:8080");
    }

    fn evidence(devtools: Vec<DevtoolsProbe>, root: bool, packages: Vec<PackageProbe>) -> DeviceEvidence {
        DeviceEvidence {
            serial: "S1".into(),
            android_release: "15".into(),
            sdk: 35,
            build_type: "user".into(),
            root_shell: root,
            adb_root_likely: false,
            packages,
            devtools,
            ..Default::default()
        }
    }

    #[test]
    fn webview_socket_wins_over_everything_on_a_locked_device() {
        let e = evidence(
            vec![DevtoolsProbe {
                pid: 13311,
                socket: "webview_devtools_remote_13311".into(),
                package: "com.hailiang.workcloud.teacher".into(),
                browser: "Chrome/124".into(),
                pages: 1,
            }],
            false,
            vec![],
        );
        let (routes, recommended, _) = assess(&e);
        assert_eq!(recommended, "cdp");
        assert!(routes[0].available && routes[0].https_plaintext);
        assert!(!routes[1].available);
        assert!(routes[3].available);
    }

    #[test]
    fn root_unlocks_full_device_plaintext() {
        let (routes, recommended, _) = assess(&evidence(vec![], true, vec![]));
        assert_eq!(recommended, "mitm-root");
        assert!(routes[1].available && routes[1].https_plaintext);
        assert!(!routes[0].available);
    }

    #[test]
    fn a_locked_device_without_webview_is_metadata_only() {
        let e = evidence(
            vec![],
            false,
            vec![PackageProbe {
                package: "com.x".into(),
                debuggable: false,
                target_sdk: 35,
            }],
        );
        let (routes, recommended, summary) = assess(&e);
        assert_eq!(recommended, "proxy-metadata");
        assert!(!routes.iter().take(3).any(|r| r.available));
        assert!(summary.contains("拿不到 HTTPS 明文"));
    }

    #[test]
    fn debuggable_alone_never_claims_https_plaintext() {
        let e = evidence(
            vec![],
            false,
            vec![PackageProbe {
                package: "com.x".into(),
                debuggable: true,
                target_sdk: 36,
            }],
        );
        let (routes, recommended, _) = assess(&e);
        let debug = routes.iter().find(|r| r.id == "mitm-debug").unwrap();
        assert!(debug.available);
        assert!(!debug.https_plaintext, "debuggable 不等于信任用户 CA");
        assert_eq!(recommended, "proxy-metadata");
    }

    // Opt-in: cd src-tauri && cargo test --lib -- --ignored --nocapture
    #[test]
    #[ignore = "需要连接一台已授权的 Android 设备"]
    fn prints_a_real_device_report() {
        let adb_path = adb::locate().expect("应找到 adb");
        let serial = adb::devices()
            .expect("adb devices 应成功")
            .into_iter()
            .find(|d| d.is_ready())
            .expect("需要一台已授权设备")
            .serial;
        let packages = list_packages(&adb_path, &serial);
        let report = probe(&adb_path, &serial, &packages).expect("体检应成功");
        eprintln!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert!(!report.routes.is_empty());
    }
}
