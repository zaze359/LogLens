//! Structured network requests, independent of how the traffic was captured.
//!
//! Three producers feed this model:
//!   * `har`  — the HAR 1.2 export every proxy (Charles, Fiddler, mitmproxy's
//!              `har_dump.py`, Chrome DevTools) can write;
//!   * `jsonl`— one compact JSON object per line, emitted by the bundled mitmproxy
//!              add-on or by any other collector;
//!   * `cdp`  — DevTools Protocol `Network.*` events from a WebView (see `cdp.rs`).
//!
//! Keeping a single `NetworkRequest` is what lets the existing filter/query machinery
//! work on traffic without turning the app into a second Wireshark.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

pub const REDACTED: &str = "[已隐藏]";

/// 默认关闭脱敏：抓包正文保留原文，便于排查。改成 true 一行即可恢复落盘脱敏。
/// 关闭时明文会进入会话、缓存与导出文件，只应在本地受信任环境下使用。
pub const REDACT_ENABLED: bool = false;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkRequest {
    pub index: u32,
    /// ISO 8601 when the source has a real date; logcat-style `MM-DD HH:MM:SS.mmm` otherwise.
    pub started_at: String,
    /// Normalised `HH:MM:SS.mmm`, the only shape the log timeline can align against.
    pub time_key: String,
    pub method: String,
    pub url: String,
    pub host: String,
    pub path: String,
    pub scheme: String,
    /// 0 means "no response yet" or "failed before a response".
    pub status: u16,
    pub status_text: String,
    pub mime_type: String,
    pub duration_ms: u32,
    pub request_size: u64,
    pub response_size: u64,
    pub request_headers: Vec<HttpHeader>,
    pub response_headers: Vec<HttpHeader>,
    pub request_body: String,
    pub response_body: String,
    /// 仅展示用的正文被上限截短时为 true；转发给设备的字节始终完整。
    /// 半截 JSON 不是合法 JSON，UI 要明确说明，不能让用户对着格式化按钮猜。
    pub response_truncated: bool,
    /// Transport-level failure (DNS, TLS, connection reset). Empty on success.
    pub error: String,
    /// `har` | `jsonl` | `cdp`
    pub source: String,
    pub redacted: bool,
}

fn sensitive_headers() -> &'static [&'static str] {
    &[
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
        "x-auth-token",
        "x-csrf-token",
        "x-xsrf-token",
    ]
}

fn sensitive_body_keys() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)("(?:token|access_token|refresh_token|password|passwd|secret|authorization|api_key|apikey|session_id)"\s*:\s*)"[^"]*""#,
        )
        .expect("static regex")
    })
}

/// Capture files routinely contain bearer tokens and session cookies, and this app
/// exports rows verbatim. Redaction is therefore applied on import, not on display:
/// a leaked credential must never reach the session, the cache or an exported file.
pub fn redact(request: &mut NetworkRequest) {
    if !REDACT_ENABLED {
        return;
    }
    redact_into(request)
}

/// 实际脱敏逻辑，单独抽出以便测试在开关关闭时仍能验证其行为。
fn redact_into(request: &mut NetworkRequest) {
    for header in request
        .request_headers
        .iter_mut()
        .chain(request.response_headers.iter_mut())
    {
        if sensitive_headers().contains(&header.name.to_ascii_lowercase().as_str()) {
            header.value = REDACTED.to_string();
            request.redacted = true;
        }
    }
    let re = sensitive_body_keys();
    for body in [&mut request.request_body, &mut request.response_body] {
        if body.is_empty() {
            continue;
        }
        let replaced = re.replace_all(body, format!("${{1}}\"{REDACTED}\"")).into_owned();
        if replaced != *body {
            *body = replaced;
            request.redacted = true;
        }
    }
    // A URL can carry a one-shot token in its query string; keep the key, hide the value.
    // `path` is derived from the URL, so it has to be rebuilt too — otherwise the token
    // would simply leak through the other column.
    let original_url = request.url.clone();
    if let Some((head, query)) = original_url.split_once('?') {
        let mut parts = Vec::new();
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let lowered = key.to_ascii_lowercase();
            if lowered.contains("token") || lowered.contains("key") || lowered.contains("sign") {
                parts.push(format!("{key}={REDACTED}"));
                request.redacted = true;
            } else {
                parts.push(format!("{key}={value}"));
            }
        }
        request.url = format!("{head}?{}", parts.join("&"));
        let (scheme, host, path) = split_url(&request.url);
        if request.scheme.is_empty() {
            request.scheme = scheme;
        }
        if request.host.is_empty() {
            request.host = host;
        }
        request.path = path;
    }
}

/// `HH:MM:SS.mmm` from an ISO 8601 timestamp. HAR uses UTC with an offset; the log
/// timeline has no date at all, so the wall-clock part is all we can align on. The
/// caller is responsible for applying a device↔host clock offset if it has one.
pub fn time_key_from_iso(iso: &str) -> String {
    let text = iso.trim();
    // HAR uses `2026-11-13T04:12:33.417Z`; a logcat-style stamp has a date and a space.
    let time = match text.split_once('T') {
        Some((_, tail)) => tail,
        None => text.rsplit_once(' ').map(|(_, tail)| tail).unwrap_or(text),
    };
    let time = time.split(['+', '-', 'Z']).next().unwrap_or(time);
    let (hms, millis) = time.split_once('.').unwrap_or((time, "0"));
    let hms: String = hms.chars().take(8).collect();
    if hms.len() < 8 {
        return String::new();
    }
    let mut ms: String = millis.chars().take(3).collect();
    while ms.len() < 3 {
        ms.push('0');
    }
    format!("{hms}.{ms}")
}

pub(crate) fn split_url(url: &str) -> (String, String, String) {
    let scheme = url.split_once("://").map(|(s, _)| s).unwrap_or("").to_string();
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let (authority, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority.rsplit('@').next().unwrap_or(authority).to_string();
    let path = format!("/{tail}");
    (scheme, host, path)
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn num_field(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn har_headers(v: Option<&Value>) -> Vec<HttpHeader> {
    v.and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| HttpHeader {
                    name: str_field(row, "name"),
                    value: str_field(row, "value"),
                })
                .filter(|h| !h.name.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn har_text(v: &Value) -> String {
    str_field(v, "text")
}

fn request_from_har_entry(entry: &Value, index: u32) -> NetworkRequest {
    let request = entry.get("request").unwrap_or(&Value::Null);
    let response = entry.get("response").unwrap_or(&Value::Null);
    let url = str_field(request, "url");
    let (scheme, host, path) = split_url(&url);
    let started = str_field(entry, "startedDateTime");
    let duration = num_field(entry, "time").max(0.0) as u32;
    let content = response.get("content");
    let post = request.get("postData");

    let mut out = NetworkRequest {
        index,
        started_at: started.clone(),
        time_key: time_key_from_iso(&started),
        method: str_field(request, "method"),
        url,
        host,
        path,
        scheme,
        status: num_field(response, "status") as u16,
        status_text: str_field(response, "statusText"),
        mime_type: content.map(|c| str_field(c, "mimeType")).unwrap_or_default(),
        duration_ms: duration,
        request_size: num_field(request, "bodySize").max(0.0) as u64,
        response_size: num_field(response, "bodySize").max(0.0) as u64,
        request_headers: har_headers(request.get("headers")),
        response_headers: har_headers(response.get("headers")),
        request_body: post.map(har_text).unwrap_or_default(),
        response_body: content.map(har_text).unwrap_or_default(),
        response_truncated: false,
        error: str_field(entry, "_error"),
        source: "har".to_string(),
        redacted: false,
    };
    if out.error.is_empty() && out.status == 0 {
        out.error = "无响应".to_string();
    }
    redact(&mut out);
    out
}

/// Parse a HAR 1.2 document. Unknown / partial entries are kept rather than dropped:
/// a failed TLS handshake has no response object but is exactly the row you need.
pub fn parse_har(text: &str) -> Result<Vec<NetworkRequest>, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("HAR 解析失败: {e}"))?;
    let entries = doc
        .get("log")
        .and_then(|l| l.get("entries"))
        .and_then(Value::as_array)
        .ok_or_else(|| "不是有效的 HAR：缺少 log.entries".to_string())?;
    Ok(entries
        .iter()
        .enumerate()
        .map(|(i, e)| request_from_har_entry(e, i as u32))
        .collect())
}

/// Parse the one-object-per-line format produced by the bundled mitmproxy add-on.
/// A malformed line is skipped instead of aborting the whole capture — a proxy that
/// crashed mid-write should still leave every earlier request analysable.
pub fn parse_jsonl(text: &str) -> Result<Vec<NetworkRequest>, String> {
    let mut out = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("跳过第 {} 行 JSONL: {e}", line_no + 1);
                continue;
            }
        };
        let mut request: NetworkRequest = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("跳过第 {} 行 JSONL: {e}", line_no + 1);
                continue;
            }
        };
        if request.url.is_empty() {
            continue;
        }
        if request.host.is_empty() {
            let (scheme, host, path) = split_url(&request.url);
            if request.scheme.is_empty() {
                request.scheme = scheme;
            }
            if request.path.is_empty() {
                request.path = path;
            }
            request.host = host;
        }
        if request.time_key.is_empty() && !request.started_at.is_empty() {
            request.time_key = time_key_from_iso(&request.started_at);
        }
        request.index = out.len() as u32;
        request.source = "jsonl".to_string();
        redact(&mut request);
        out.push(request);
    }
    if out.is_empty() {
        return Err("文件里没有可识别的网络请求".to_string());
    }
    Ok(out)
}

/// Auto-detect a capture file. A HAR is a single JSON object with `log.entries`;
/// anything else is treated as JSONL, which is also what our own collector writes.
pub fn parse_capture_file(text: &str) -> Result<Vec<NetworkRequest>, String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        if let Ok(doc) = serde_json::from_str::<Value>(trimmed) {
            if doc.pointer("/log/entries").is_some() {
                return parse_har(trimmed);
            }
        }
    }
    parse_jsonl(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAR: &str = r#"{
      "log": {"version": "1.2", "entries": [
        {"startedDateTime": "2026-11-13T04:12:33.417Z", "time": 187.5,
         "request": {"method": "POST", "url": "https://api.example.com/v1/homework?token=abc123",
           "headers": [{"name": "Authorization", "value": "Bearer secret-token"}],
           "bodySize": 42, "postData": {"text": "{\"title\":\"作业\",\"password\":\"p@ss\"}"}},
         "response": {"status": 200, "statusText": "OK", "bodySize": 913,
           "headers": [{"name": "Set-Cookie", "value": "sid=xyz"}],
           "content": {"mimeType": "application/json", "text": "{\"code\":0,\"access_token\":\"leak\"}"}}},
        {"startedDateTime": "2026-11-13T04:12:34.001+08:00", "time": 3000,
         "request": {"method": "GET", "url": "https://cdn.example.com/a.png"},
         "response": {"status": 0, "statusText": ""}, "_error": "net::ERR_TIMED_OUT"}
      ]}
    }"#;

    #[test]
    fn parses_har_entries_into_structured_requests() {
        let rows = parse_har(HAR).expect("HAR 应解析成功");
        assert_eq!(rows.len(), 2);
        let first = &rows[0];
        assert_eq!(first.method, "POST");
        assert_eq!(first.host, "api.example.com");
        // 脱敏默认关闭，URL query 保留原值。
        assert_eq!(first.path, "/v1/homework?token=abc123");
        assert_eq!(first.scheme, "https");
        assert_eq!(first.status, 200);
        assert_eq!(first.duration_ms, 187);
        assert_eq!(first.mime_type, "application/json");
        assert_eq!(first.time_key, "04:12:33.417");
        assert_eq!(first.source, "har");

        let second = &rows[1];
        assert_eq!(second.status, 0);
        assert_eq!(second.error, "net::ERR_TIMED_OUT");
        assert_eq!(second.duration_ms, 3000);
    }

    #[test]
    fn redaction_is_off_by_default() {
        let rows = parse_har(HAR).unwrap();
        let first = &rows[0];
        assert!(!first.redacted);
        assert_eq!(first.request_headers[0].value, "Bearer secret-token");
        assert!(first.request_body.contains("p@ss"));
        assert!(first.response_body.contains("leak"));
        assert!(first.url.contains("abc123"));
    }

    #[test]
    fn redact_into_hides_credentials_when_enabled() {
        // 直接测脱敏逻辑本身：开关关闭时生产路径不调用它，但能力保留，改 REDACT_ENABLED 即可恢复。
        let mut row = NetworkRequest {
            request_headers: vec![
                HttpHeader { name: "Authorization".into(), value: "Bearer secret-token".into() },
            ],
            response_headers: vec![
                HttpHeader { name: "Set-Cookie".into(), value: "sid=xyz".into() },
            ],
            request_body: "{\"title\":\"作业\",\"password\":\"p@ss\"}".into(),
            response_body: "{\"code\":0,\"access_token\":\"leak\"}".into(),
            url: "https://api.example.com/v1/homework?token=abc123".into(),
            ..Default::default()
        };
        let (_, host, path) = split_url(&row.url);
        row.host = host;
        redact_into(&mut row);
        assert!(row.redacted);
        assert_eq!(row.request_headers[0].value, REDACTED);
        assert_eq!(row.response_headers[0].value, REDACTED);
        assert!(row.request_body.contains(REDACTED));
        assert!(!row.request_body.contains("p@ss"));
        assert!(row.response_body.contains(REDACTED));
        assert!(!row.response_body.contains("leak"));
        assert!(!row.url.contains("abc123"));
        assert_eq!(path, "/v1/homework?token=abc123");
    }

    #[test]
    fn detects_har_and_jsonl_automatically() {
        assert_eq!(parse_capture_file(HAR).unwrap().len(), 2);
        let line = r#"{"method":"GET","url":"https://example.com/a","status":204,"startedAt":"2026-11-13T04:12:33.417Z"}"#;
        let rows = parse_capture_file(&format!("{line}\nnot-json\n{line}\n")).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].host, "example.com");
        assert_eq!(rows[0].path, "/a");
        assert_eq!(rows[1].index, 1);
    }

    #[test]
    fn rejects_a_file_with_no_requests() {
        assert!(parse_jsonl("").is_err());
        assert!(parse_har(r#"{"log":{"entries":[]}}"#).unwrap().is_empty());
    }

    #[test]
    fn time_key_handles_offsets_and_missing_millis() {
        assert_eq!(time_key_from_iso("2026-11-13T04:12:33.417Z"), "04:12:33.417");
        assert_eq!(time_key_from_iso("2026-11-13T04:12:33+08:00"), "04:12:33.000");
        assert_eq!(time_key_from_iso("11-13 04:12:33.417"), "04:12:33.417");
    }
}
