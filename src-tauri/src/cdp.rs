//! Minimal Chrome DevTools Protocol client, deliberately dependency-free.
//!
//! Android's WebView exposes a `localabstract:webview_devtools_remote_<pid>` socket. Once
//! `adb forward` maps it to a local TCP port, that endpoint speaks ordinary HTTP for
//! `/json/version` and `/json/list`, and WebSocket for the DevTools protocol itself.
//! `Network.enable` then yields **HTTPS request and response plaintext** for anything the
//! WebView loads — no root, no CA certificate, no change to the app under test.
//!
//! Why hand-rolled instead of `tokio-tungstenite`: the project ships only `serde`,
//! `serde_json` and `regex`. Adding a WebSocket stack means a new dependency and a
//! network fetch on every clean build, for a protocol where we need exactly three
//! things — an HTTP GET, a masked text frame out, and unmasked text frames in.
//!
//! Known scope limits:
//!   * only WebView traffic. Native OkHttp / Cronet requests never cross this channel.
//!   * response bodies are not collected unless `Network.getResponseBody` is issued;
//!     headers, status, size and timing are available for every request.

use crate::network::{redact, HttpHeader, NetworkRequest};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default)]
pub struct CdpVersion {
    pub browser: String,
    pub package: String,
    /// Kept for diagnostics; capture decisions only need the browser and package.
    #[allow(dead_code)]
    pub protocol: String,
}

#[derive(Debug, Clone, Default)]
pub struct CdpTarget {
    /// Kept for diagnostics even though the UI only needs the WebSocket path.
    #[allow(dead_code)]
    pub id: String,
    pub title: String,
    pub url: String,
    /// Path part of `webSocketDebuggerUrl`, e.g. `/devtools/page/ABC`.
    pub ws_path: String,
    pub kind: String,
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Path of a `ws://host:port/...` URL. The CDP HTTP and WS endpoints share one port, so
/// only the path is needed once `adb forward` is in place.
pub fn ws_path(url: &str) -> String {
    url.split_once("://")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.find('/').map(|i| &rest[i..]))
        .unwrap_or("/devtools/page")
        .to_string()
}

pub fn parse_version(text: &str) -> CdpVersion {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    CdpVersion {
        browser: str_field(&v, "Browser"),
        package: str_field(&v, "Android-Package"),
        protocol: str_field(&v, "Protocol-Version"),
    }
}

pub fn parse_targets(text: &str) -> Vec<CdpTarget> {
    let rows: Vec<Value> = serde_json::from_str(text).unwrap_or_default();
    rows.iter()
        .map(|row| CdpTarget {
            id: str_field(row, "id"),
            title: str_field(row, "title"),
            url: str_field(row, "url"),
            ws_path: ws_path(&str_field(row, "webSocketDebuggerUrl")),
            kind: str_field(row, "type"),
        })
        .collect()
}

fn decode_chunked(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor < body.len() {
        let Some(rel) = body[cursor..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let line = &body[cursor..cursor + rel];
        let size_text = String::from_utf8_lossy(line).split(';').next().unwrap_or("0").trim().to_string();
        let size = usize::from_str_radix(&size_text, 16).unwrap_or(0);
        cursor += rel + 2;
        if size == 0 || cursor + size > body.len() {
            break;
        }
        out.extend_from_slice(&body[cursor..cursor + size]);
        cursor += size + 2; // trailing CRLF
    }
    out
}

/// One HTTP GET with `Connection: close`.
///
/// HTTP/1.1 is mandatory, not optional: Android's WebView devtools HTTP server silently
/// drops HTTP/1.0 requests (it answers zero bytes and leaves the socket open). Verified
/// against Chrome/124 WebView on Android 15. Responses carry Content-Length; the chunked
/// decoder is a fallback for other Chromium builds.
pub fn http_get(port: u16, path: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| format!("无法连接 CDP 端口 {port}: {e}"))?;
    stream.set_read_timeout(Some(TIMEOUT)).ok();
    stream.set_write_timeout(Some(TIMEOUT)).ok();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("CDP 请求失败: {e}"))?;

    // Chrome's devtools server sends Content-Length but does not always close the socket
    // promptly, so `read_to_end` would sit until the read timeout. Read the headers, then
    // exactly the announced body length instead.
    let mut raw = Vec::new();
    let mut buf = [0u8; 1024];
    let header_end = loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => return Err("CDP 在响应头结束前关闭了连接".to_string()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err("CDP 响应头读取超时".to_string())
            }
            Err(e) => return Err(format!("CDP 响应读取失败: {e}")),
        };
        raw.extend_from_slice(&buf[..n]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if raw.len() > 64 * 1024 {
            return Err("CDP 响应头过大".to_string());
        }
    };
    let headers = String::from_utf8_lossy(&raw[..header_end]).to_ascii_lowercase();
    if !headers.contains(" 200 ") {
        return Err(format!("CDP 返回非 200: {}", headers.lines().next().unwrap_or("")));
    }
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok());
    let mut body = raw[header_end + 4..].to_vec();
    loop {
        if let Some(len) = content_length {
            if body.len() >= len {
                body.truncate(len);
                break;
            }
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut => break,
            Err(e) => return Err(format!("CDP 响应体读取失败: {e}")),
        }
    }
    if headers.contains("transfer-encoding: chunked") {
        body = decode_chunked(&body);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

pub fn version(port: u16) -> Result<CdpVersion, String> {
    Ok(parse_version(&http_get(port, "/json/version")?))
}

pub fn targets(port: u16) -> Result<Vec<CdpTarget>, String> {
    Ok(parse_targets(&http_get(port, "/json/list")?))
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

enum FrameRead {
    Complete,
    Idle,
    Closed,
}

/// RFC 6455 client with just enough surface for CDP: masked text frames out, text / ping /
/// close frames in, and continuation frames assembled into one message.
pub struct WsClient {
    stream: TcpStream,
    closed: bool,
}

impl WsClient {
    pub fn connect(port: u16, path: &str) -> Result<Self, String> {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .map_err(|e| format!("无法连接 CDP WebSocket: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .map_err(|e| format!("设置读超时失败: {e}"))?;
        // CDP only checks that the key is valid base64 of a 16-byte nonce. Deriving it
        // from the process id + nanoseconds keeps it fresh without pulling in `rand`.
        let seed = format!("{}-{}", std::process::id(), std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        let mut nonce = [0u8; 16];
        for (i, byte) in seed.bytes().enumerate() {
            nonce[i % 16] ^= byte;
        }
        let handshake = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            base64(&nonce)
        );
        stream
            .write_all(handshake.as_bytes())
            .map_err(|e| format!("WebSocket 握手失败: {e}"))?;

        let mut response = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = stream.read(&mut byte).map_err(|e| format!("读取握手响应失败: {e}"))?;
            if n == 0 {
                return Err("WebSocket 握手时连接被关闭".to_string());
            }
            response.push(byte[0]);
            if response.ends_with(b"\r\n\r\n") {
                break;
            }
            if response.len() > 16 * 1024 {
                return Err("WebSocket 握手响应过大".to_string());
            }
        }
        let head = String::from_utf8_lossy(&response);
        if !head.contains("101") {
            return Err(format!("WebSocket 升级被拒绝: {}", head.lines().next().unwrap_or("")));
        }
        Ok(Self { stream, closed: false })
    }

    pub fn send_text(&mut self, text: &str) -> Result<(), String> {
        let payload = text.as_bytes();
        let mask = [0x37u8, 0xfa, 0x21, 0x3d];
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x81); // FIN + text
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if len < 65536 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        for (i, b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i % 4]);
        }
        self.stream
            .write_all(&frame)
            .map_err(|e| format!("WebSocket 发送失败: {e}"))
    }

    /// A 1s read timeout is how `stop()` gets a responsive shutdown without a dedicated
    /// signal. `Idle` is only produced while waiting for the *first* byte of a frame: once
    /// a frame has started we must wait for the rest of it rather than drop half a frame.
    fn read_exact(&mut self, buf: &mut [u8], first: bool) -> Result<FrameRead, String> {
        let mut filled = 0usize;
        while filled < buf.len() {
            match self.stream.read(&mut buf[filled..]) {
                Ok(0) => return Ok(FrameRead::Closed),
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if first && filled == 0 {
                        return Ok(FrameRead::Idle);
                    }
                }
                Err(e) => return Err(format!("WebSocket 读取失败: {e}")),
            }
        }
        Ok(FrameRead::Complete)
    }

    fn read_rest(&mut self, buf: &mut [u8]) -> Result<bool, String> {
        match self.read_exact(buf, false)? {
            FrameRead::Complete => Ok(true),
            _ => {
                self.closed = true;
                Ok(false)
            }
        }
    }

    /// `Ok(None)` means the peer closed; `Ok(Some(""))` means "no frame this second /
    /// only a control frame", which callers use to check their stop flag.
    pub fn recv_text(&mut self) -> Result<Option<String>, String> {
        if self.closed {
            return Ok(None);
        }
        let mut head = [0u8; 2];
        match self.read_exact(&mut head, true)? {
            FrameRead::Idle => return Ok(Some(String::new())),
            FrameRead::Closed => {
                self.closed = true;
                return Ok(None);
            }
            FrameRead::Complete => {}
        }
        let fin = head[0] & 0x80 != 0;
        let opcode = head[0] & 0x0f;
        let masked = head[1] & 0x80 != 0;
        let mut len = (head[1] & 0x7f) as usize;
        if len == 126 {
            let mut ext = [0u8; 2];
            if !self.read_rest(&mut ext)? { return Ok(None); }
            len = u16::from_be_bytes(ext) as usize;
        } else if len == 127 {
            let mut ext = [0u8; 8];
            if !self.read_rest(&mut ext)? { return Ok(None); }
            len = u64::from_be_bytes(ext) as usize;
        }
        if len > 16 * 1024 * 1024 {
            return Err("WebSocket 帧过大".to_string());
        }
        let mut mask = [0u8; 4];
        if masked && !self.read_rest(&mut mask)? {
            return Ok(None);
        }
        let mut payload = vec![0u8; len];
        if !self.read_rest(&mut payload)? {
            return Ok(None);
        }
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        match opcode {
            0x8 => {
                self.closed = true;
                Ok(None)
            }
            0x9 => {
                let mut pong = vec![0x8a, len as u8];
                pong.extend_from_slice(&payload);
                let _ = self.stream.write_all(&pong);
                Ok(Some(String::new()))
            }
            0xa => Ok(Some(String::new())),
            _ => {
                let mut text = String::from_utf8_lossy(&payload).into_owned();
                while !fin {
                    if let Some(next) = self.recv_text()? {
                        text.push_str(&next);
                    } else {
                        break;
                    }
                }
                Ok(Some(text))
            }
        }
    }
}

/// `HH:MM:SS.mmm` from a CDP `wallTime` (Unix seconds, UTC) plus the device's timezone
/// offset. Logcat timestamps carry no date, so the wall-clock part is the shared key.
pub fn hms_from_wall_time(seconds: f64, offset_secs: i32) -> String {
    if seconds <= 0.0 {
        return String::new();
    }
    let total_ms = (seconds * 1000.0) as i64 + offset_secs as i64 * 1000;
    let day_ms = (total_ms % 86_400_000 + 86_400_000) % 86_400_000;
    let h = day_ms / 3_600_000;
    let m = (day_ms % 3_600_000) / 60_000;
    let s = (day_ms % 60_000) / 1000;
    let ms = day_ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Per-connection state machine. `requestId` is the join key across the four CDP events.
pub struct CdpState {
    pending: HashMap<String, NetworkRequest>,
    starts: HashMap<String, f64>,
    completed: Vec<NetworkRequest>,
    offset_secs: i32,
    next_index: u32,
}

impl CdpState {
    pub fn new(offset_secs: i32) -> Self {
        Self {
            pending: HashMap::new(),
            starts: HashMap::new(),
            completed: Vec::new(),
            offset_secs,
            next_index: 0,
        }
    }

    pub fn take_completed(&mut self) -> Vec<NetworkRequest> {
        std::mem::take(&mut self.completed)
    }

    #[allow(dead_code)]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    fn finish(&mut self, id: &str) {
        if let Some(mut request) = self.pending.remove(id) {
            request.index = self.next_index;
            self.next_index += 1;
            redact(&mut request);
            self.completed.push(request);
        }
        self.starts.remove(id);
    }

    /// Returns true when the event was a `Network.*` notification we understood.
    pub fn apply(&mut self, message: &Value) -> bool {
        let method = str_field(message, "method");
        let Some(params) = message.get("params") else {
            return false;
        };
        let id = str_field(params, "requestId");
        if id.is_empty() {
            return false;
        }
        match method.as_str() {
            "Network.requestWillBeSent" => {
                let req = params.get("request").unwrap_or(&Value::Null);
                let url = str_field(req, "url");
                let (scheme, host, path) = crate::network::split_url(&url);
                let wall = params.get("wallTime").and_then(Value::as_f64).unwrap_or(0.0);
                let stamp = hms_from_wall_time(wall, self.offset_secs);
                let headers: Vec<HttpHeader> = req
                    .get("headers")
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| HttpHeader {
                                name: k.clone(),
                                value: v.as_str().unwrap_or("").to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.starts.insert(id.clone(), params.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0));
                self.pending.insert(
                    id,
                    NetworkRequest {
                        started_at: stamp.clone(),
                        time_key: stamp,
                        method: str_field(req, "method"),
                        url,
                        host,
                        path,
                        scheme,
                        request_headers: headers,
                        request_body: str_field(req, "postData"),
                        source: "cdp".to_string(),
                        ..Default::default()
                    },
                );
                true
            }
            "Network.responseReceived" => {
                let wall = params.get("wallTime").and_then(Value::as_f64).unwrap_or(0.0);
                if let Some(request) = self.pending.get_mut(&id) {
                    let resp = params.get("response").unwrap_or(&Value::Null);
                    request.status = resp.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
                    request.status_text = str_field(resp, "statusText");
                    request.mime_type = str_field(resp, "mimeType");
                    request.response_headers = resp
                        .get("headers")
                        .and_then(Value::as_object)
                        .map(|m| {
                            m.iter()
                                .map(|(k, v)| HttpHeader {
                                    name: k.clone(),
                                    value: v.as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if request.time_key.is_empty() {
                        let stamp = hms_from_wall_time(wall, self.offset_secs);
                        request.started_at = stamp.clone();
                        request.time_key = stamp;
                    }
                }
                true
            }
            "Network.dataReceived" => {
                if let Some(request) = self.pending.get_mut(&id) {
                    request.response_size += params.get("dataLength").and_then(Value::as_u64).unwrap_or(0);
                }
                true
            }
            "Network.loadingFinished" => {
                let end = params.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0);
                let start = self.starts.get(&id).copied().unwrap_or(0.0);
                if let Some(request) = self.pending.get_mut(&id) {
                    request.duration_ms = ((end - start).max(0.0) * 1000.0) as u32;
                    if request.response_size == 0 {
                        request.response_size = params
                            .get("encodedDataLength")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0)
                            .max(0.0) as u64;
                    }
                }
                self.finish(&id);
                true
            }
            "Network.loadingFailed" => {
                if let Some(request) = self.pending.get_mut(&id) {
                    request.error = str_field(params, "errorText");
                    if request.error.is_empty() {
                        request.error = "请求失败".to_string();
                    }
                }
                self.finish(&id);
                true
            }
            _ => false,
        }
    }

    /// Requests still in flight when the capture stops. Keeping them (instead of dropping)
    /// preserves the "a request was sent but never answered" evidence.
    pub fn flush(&mut self) -> Vec<NetworkRequest> {
        let ids: Vec<String> = self.pending.keys().cloned().collect();
        for id in ids {
            if let Some(request) = self.pending.get_mut(&id) {
                if request.error.is_empty() {
                    request.error = "抓取结束时仍无响应".to_string();
                }
            }
            self.finish(&id);
        }
        self.take_completed()
    }
}

pub fn enable_network_message(id: u64) -> String {
    json!({
        "id": id,
        "method": "Network.enable",
        "params": { "maxTotalBufferSize": 20_000_000, "maxPostDataSize": 65536 }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_and_targets() {
        let v = parse_version(r#"{"Browser":"Chrome/124.0.6367.219","Android-Package":"com.example.sample.teacher","Protocol-Version":"1.3"}"#);
        assert_eq!(v.package, "com.example.sample.teacher");
        assert_eq!(v.browser, "Chrome/124.0.6367.219");

        let t = parse_targets(r#"[{"id":"P1","type":"page","title":"作业","url":"https://x.cn/a","webSocketDebuggerUrl":"ws://127.0.0.1:19222/devtools/page/P1"}]"#);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].ws_path, "/devtools/page/P1");
        assert_eq!(t[0].title, "作业");
    }

    #[test]
    fn base64_and_chunked_match_reference_values() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(&[0u8; 16]).len(), 24);
        assert_eq!(decode_chunked(b"4\r\nWiki\r\n0\r\n\r\n"), b"Wiki");
    }

    #[test]
    fn converts_wall_time_using_the_device_offset() {
        // 2026-11-13 04:12:33.417 UTC
        let epoch = 1763007153.417;
        assert_eq!(hms_from_wall_time(epoch, 0), "04:12:33.417");
        assert_eq!(hms_from_wall_time(epoch, 8 * 3600), "12:12:33.417");
        assert_eq!(hms_from_wall_time(0.0, 0), "");
    }

    fn event(method: &str, params: Value) -> Value {
        json!({ "method": method, "params": params })
    }

    #[test]
    fn assembles_one_request_from_four_cdp_events() {
        let mut state = CdpState::new(8 * 3600);
        assert!(state.apply(&event(
            "Network.requestWillBeSent",
            json!({
                "requestId": "r1",
                "timestamp": 10.0,
                "wallTime": 1763007153.417,
                "request": {
                    "url": "https://api.example.com/v1/hw?token=abc",
                    "method": "POST",
                    "headers": {"Authorization": "Bearer secret"},
                    "postData": "{\"password\":\"p\"}"
                }
            })
        )));
        state.apply(&event(
            "Network.responseReceived",
            json!({
                "requestId": "r1",
                "response": {"status": 200, "statusText": "OK", "mimeType": "application/json",
                             "headers": {"Set-Cookie": "sid=x"}}
            }),
        ));
        state.apply(&event("Network.dataReceived", json!({"requestId": "r1", "dataLength": 512})));
        state.apply(&event("Network.loadingFinished", json!({"requestId": "r1", "timestamp": 10.25})));

        let rows = state.take_completed();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.status, 200);
        assert_eq!(row.method, "POST");
        assert_eq!(row.host, "api.example.com");
        assert_eq!(row.duration_ms, 250);
        assert_eq!(row.response_size, 512);
        assert_eq!(row.time_key, "12:12:33.417");
        assert_eq!(row.source, "cdp");
        // 脱敏默认关闭：原文保留（含 token / Authorization / password）。
        assert!(!row.redacted);
        assert!(row.url.contains("abc"));
        assert_eq!(row.request_headers[0].value, "Bearer secret");
        assert!(row.request_body.contains("password"));
    }

    #[test]
    fn failed_and_unfinished_requests_are_kept() {
        let mut state = CdpState::new(0);
        state.apply(&event("Network.requestWillBeSent", json!({
            "requestId": "a", "timestamp": 1.0,
            "request": {"url": "https://x.cn/a", "method": "GET"}
        })));
        state.apply(&event("Network.loadingFailed", json!({
            "requestId": "a", "errorText": "net::ERR_TIMED_OUT", "timestamp": 4.0
        })));
        state.apply(&event("Network.requestWillBeSent", json!({
            "requestId": "b", "timestamp": 2.0,
            "request": {"url": "https://x.cn/b", "method": "GET"}
        })));
        assert_eq!(state.pending_count(), 1);
        let rows = state.flush();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].error, "net::ERR_TIMED_OUT");
        assert_eq!(rows[1].error, "抓取结束时仍无响应");
        assert_eq!(state.pending_count(), 0);
    }

    #[test]
    fn ignores_unrelated_events_and_malformed_messages() {
        let mut state = CdpState::new(0);
        assert!(!state.apply(&json!({"id": 1, "result": {}})));
        assert!(!state.apply(&event("Page.loadEventFired", json!({"requestId": "x"}))));
        assert!(!state.apply(&event("Network.requestWillBeSent", json!({}))));
        assert_eq!(state.pending_count(), 0);
    }

    #[test]
    fn enable_message_has_the_expected_shape() {
        let msg: Value = serde_json::from_str(&enable_network_message(7)).unwrap();
        assert_eq!(msg["id"], 7);
        assert_eq!(msg["method"], "Network.enable");
    }
}
