//! Built-in HTTP proxy for **native** (non-WebView) device traffic.
//!
//! WebView CDP sees H5 traffic but never an OkHttp call. This proxy closes that gap
//! without root, without a CA certificate and without changing the app: the desktop
//! listens on its LAN address, `settings put global http_proxy` points the device at it,
//! and every connection becomes a [`NetworkRequest`].
//!
//! What can and cannot be seen on a locked-down Android 15 device:
//!
//! * plain HTTP — full request/response: URL, headers, body, status, timing. The target
//!   apps here declare `cleartextTrafficPermitted=true`, so this is real coverage.
//! * HTTPS — the `CONNECT` target, the TLS **SNI** from the ClientHello, connection
//!   duration and byte counts. The payload stays encrypted: decrypting it would require
//!   a CA the app trusts, which a release build without `debug-overrides` will refuse.
//!
//! Deliberately dependency-free (no TLS stack, no HTTP library): this is a transport
//! relay, not a man-in-the-middle. A failed CONNECT is still recorded as evidence.

use crate::adb;
use crate::cdp::hms_from_wall_time;
use crate::netcapture::{NetworkCaptureStatus, ProxyGuard};
use crate::network::{redact, split_url, HttpHeader, NetworkRequest};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_BODY: usize = 64 * 1024;
const READ_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxyCaptureInfo {
    /// Address the device is pointed at, e.g. `172.21.33.28:8899`.
    pub endpoint: String,
    pub device_ip: String,
    pub allowed_sources: String,
}

#[derive(Default)]
struct ProxyBuffer {
    pending: Vec<NetworkRequest>,
    connections: u32,
    finished: bool,
    error: String,
}

/// Read the TLS Server Name Indication out of a ClientHello.
///
/// Layout: TLS record (5) → handshake header (4) → version (2) + random (32) +
/// session id + cipher suites + compression methods → extensions, where extension
/// `0x0000` carries the ServerNameList. Returns None for anything that is not a
/// parseable ClientHello; a proxy must never break traffic because of its own parser.
pub fn parse_sni(data: &[u8]) -> Option<String> {
    if data.len() < 43 || data[0] != 0x16 || data[5] != 0x01 {
        return None;
    }
    let mut p = 43usize; // 5 record + 4 handshake + 2 version + 32 random
    if p > data.len() {
        return None;
    }
    let sid_len = data[p] as usize;
    p += 1 + sid_len;
    if p + 2 > data.len() {
        return None;
    }
    let cipher_len = u16::from_be_bytes([data[p], data[p + 1]]) as usize;
    p += 2 + cipher_len;
    if p >= data.len() {
        return None;
    }
    let compression_len = data[p] as usize;
    p += 1 + compression_len;
    if p + 2 > data.len() {
        return None;
    }
    let extensions_len = u16::from_be_bytes([data[p], data[p + 1]]) as usize;
    p += 2;
    let end = (p + extensions_len).min(data.len());
    while p + 4 <= end {
        let ext_type = u16::from_be_bytes([data[p], data[p + 1]]);
        let ext_len = u16::from_be_bytes([data[p + 2], data[p + 3]]) as usize;
        let body = p + 4;
        if body + ext_len > end {
            break;
        }
        if ext_type == 0x0000 && ext_len >= 5 {
            let mut q = body + 2; // skip ServerNameList length
            while q + 3 <= body + ext_len {
                let name_type = data[q];
                let name_len = u16::from_be_bytes([data[q + 1], data[q + 2]]) as usize;
                q += 3;
                if q + name_len > body + ext_len {
                    break;
                }
                if name_type == 0 {
                    let name = String::from_utf8_lossy(&data[q..q + name_len]).into_owned();
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
                q += name_len;
            }
        }
        p = body + ext_len;
    }
    None
}

/// UDP `connect` performs no I/O; it only asks the OS which local address would be
/// used to reach the peer. Pointing it at the device picks the right LAN interface.
pub fn local_lan_ip(peer: Option<Ipv4Addr>) -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    let target = SocketAddr::V4(SocketAddrV4::new(
        peer.unwrap_or(Ipv4Addr::new(8, 8, 8, 8)),
        80,
    ));
    socket.connect(target).ok()?;
    match socket.local_addr().ok()? {
        SocketAddr::V4(addr) => Some(*addr.ip()),
        _ => None,
    }
}

pub fn parse_device_ip(text: &str) -> Option<Ipv4Addr> {
    text.lines()
        .flat_map(|line| line.split_whitespace())
        .filter_map(|token| token.split('/').next())
        .find_map(|token| token.parse::<Ipv4Addr>().ok())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

pub fn device_ip(adb_path: &Path, serial: &str) -> Option<Ipv4Addr> {
    let wlan = adb::shell_probe(adb_path, serial, "ip -f inet addr show wlan0 | grep 'inet '");
    if let Some(ip) = parse_device_ip(&wlan) {
        return Some(ip);
    }
    let prop = adb::shell_probe(adb_path, serial, "getprop dhcp.wlan0.ipaddress");
    parse_device_ip(&prop)
}

fn now_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64
        / 1000.0
}

fn header_pairs(lines: &[&str]) -> Vec<HttpHeader> {
    lines
        .iter()
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| HttpHeader {
            name: k.trim().to_string(),
            value: v.trim().to_string(),
        })
        .filter(|h| !h.name.is_empty())
        .collect()
}

fn header_value(headers: &[HttpHeader], name: &str) -> String {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.clone())
        .unwrap_or_default()
}

fn is_textual(mime: &str) -> bool {
    mime.contains("json") || mime.contains("text") || mime.contains("xml")
        || mime.contains("javascript") || mime.contains("urlencoded")
}

/// Read exactly up to `\r\n\r\n`, one byte at a time. A BufReader would prefetch bytes
/// belonging to the TLS handshake or request body, which breaks CONNECT relaying.
fn read_head(stream: &mut TcpStream) -> Result<String, String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    let mut matched = 0usize;
    const END: &[u8] = b"\r\n\r\n";
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok();
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Err("连接在请求头结束前关闭".to_string()),
            Ok(_) => {
                head.push(byte[0]);
                if byte[0] == END[matched] {
                    matched += 1;
                    if matched == END.len() {
                        break;
                    }
                } else {
                    matched = if byte[0] == END[0] { 1 } else { 0 };
                }
                if head.len() > 64 * 1024 {
                    return Err("请求头过大".to_string());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err("读取请求头超时".to_string())
            }
            Err(e) => return Err(format!("读取请求头失败: {e}")),
        }
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

fn push_request(shared: &Arc<Mutex<ProxyBuffer>>, request: NetworkRequest) {
    if let Ok(mut buffer) = shared.lock() {
        buffer.pending.push(request);
        buffer.connections += 1;
    }
}

fn make_request(offset_secs: i32, started: f64) -> NetworkRequest {
    let stamp = hms_from_wall_time(started, offset_secs);
    NetworkRequest {
        started_at: stamp.clone(),
        time_key: stamp,
        source: "proxy".to_string(),
        ..Default::default()
    }
}

/// Plain HTTP: parse, forward unchanged in origin-form, return the real response to the
/// client, and record both directions. `Connection: close` on the upstream keeps the
/// response framing simple; OkHttp will just open a new connection next time.
fn handle_http(
    mut client: TcpStream,
    request_line: &str,
    head_lines: &[&str],
    shared: &Arc<Mutex<ProxyBuffer>>,
    offset_secs: i32,
) -> Result<(), String> {
    let started = now_millis();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let headers = header_pairs(head_lines);
    let content_length: usize = header_value(&headers, "content-length")
        .parse()
        .unwrap_or(0)
        .min(4 * 1024 * 1024);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        client
            .read_exact(&mut body)
            .map_err(|e| format!("读取请求体失败: {e}"))?;
    }

    let (scheme, host_port, path) = if target.starts_with("http://") || target.starts_with("https://") {
        let (s, h, p) = split_url(&target);
        (s, h, p)
    } else {
        ("http".to_string(), header_value(&headers, "host"), target.clone())
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(80)),
        None => (host_port.clone(), 80),
    };
    if host.is_empty() {
        return Err("请求缺少 Host".to_string());
    }

    let mut upstream = TcpStream::connect((host.as_str(), port))
        .map_err(|e| format!("连接 {host}:{port} 失败: {e}"))?;
    upstream.set_read_timeout(Some(Duration::from_secs(20))).ok();
    upstream.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let mut forward = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for h in &headers {
        let lower = h.name.to_ascii_lowercase();
        if lower == "host" || lower == "connection" || lower.starts_with("proxy-")
            || lower == "content-length"
        {
            continue;
        }
        forward.push_str(&format!("{}: {}\r\n", h.name, h.value));
    }
    if !body.is_empty() {
        forward.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    forward.push_str("\r\n");
    upstream
        .write_all(forward.as_bytes())
        .and_then(|_| upstream.write_all(&body))
        .map_err(|e| format!("转发请求失败: {e}"))?;

    let mut response = Vec::new();
    upstream
        .read_to_end(&mut response)
        .map_err(|e| format!("读取上游响应失败: {e}"))?;
    // The app must receive the untouched bytes regardless of how well we parse them.
    let _ = client.write_all(&response);
    let _ = client.flush();

    let split = response.windows(4).position(|w| w == b"\r\n\r\n");
    let (response_head, response_body) = match split {
        Some(i) => (&response[..i], &response[i + 4..]),
        None => (&response[..], &[][..]),
    };
    let head_text = String::from_utf8_lossy(response_head);
    let mut lines = head_text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status_parts: Vec<&str> = status_line.splitn(3, ' ').collect();
    let status = status_parts.get(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
    let status_text = status_parts.get(2).unwrap_or(&"").to_string();
    let response_headers = header_pairs(&lines.collect::<Vec<_>>());
    let mime = header_value(&response_headers, "content-type");

    let url = if target.starts_with("http") {
        target.clone()
    } else {
        format!("http://{host_port}{path}")
    };
    let mut request = make_request(offset_secs, started);
    request.method = method;
    request.url = url;
    request.host = host;
    request.path = path;
    request.scheme = scheme;
    request.status = status;
    request.status_text = status_text;
    request.mime_type = mime.clone();
    request.request_size = body.len() as u64;
    request.response_size = response_body.len() as u64;
    request.request_headers = headers;
    request.response_headers = response_headers;
    if is_textual(&mime) {
        request.request_body = String::from_utf8_lossy(&body[..body.len().min(MAX_BODY)]).into_owned();
        request.response_body = String::from_utf8_lossy(
            &response_body[..response_body.len().min(MAX_BODY)],
        )
        .into_owned();
    }
    request.duration_ms = ((now_millis() - started).max(0.0) * 1000.0) as u32;
    redact(&mut request);
    push_request(shared, request);
    Ok(())
}

fn relay(
    mut from: TcpStream,
    mut to: TcpStream,
    counter: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    sni_slot: Option<Arc<Mutex<Option<String>>>>,
) {
    from.set_read_timeout(Some(READ_TIMEOUT)).ok();
    to.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let mut buf = [0u8; 16 * 1024];
    let mut first = true;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if first {
                    if let Some(slot) = &sni_slot {
                        if let Some(name) = parse_sni(&buf[..n]) {
                            if let Ok(mut guard) = slot.lock() {
                                *guard = Some(name);
                            }
                        }
                    }
                    first = false;
                }
                counter.fetch_add(n as u64, Ordering::Relaxed);
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => break,
        }
    }
    let _ = to.shutdown(Shutdown::Write);
}

/// HTTPS: establish the tunnel, relay bytes in both directions, and record the domain
/// from CONNECT plus the TLS SNI. Nothing here attempts to decrypt application data.
fn handle_connect(
    mut client: TcpStream,
    request_line: &str,
    head_lines: &[&str],
    shared: &Arc<Mutex<ProxyBuffer>>,
    stop: Arc<AtomicBool>,
    offset_secs: i32,
) -> Result<(), String> {
    let started = now_millis();
    let target = request_line.split_whitespace().nth(1).unwrap_or("");
    let (host, port) = match target.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(443)),
        None => (target.to_string(), 443),
    };
    if host.is_empty() {
        return Err("CONNECT 缺少目标主机".to_string());
    }
    let upstream = TcpStream::connect((host.as_str(), port))
        .map_err(|e| format!("连接 {host}:{port} 失败: {e}"))?;
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .map_err(|e| format!("回复 CONNECT 失败: {e}"))?;

    let up_bytes = Arc::new(AtomicU64::new(0));
    let down_bytes = Arc::new(AtomicU64::new(0));
    let sni_slot = Arc::new(Mutex::new(None::<String>));
    let client_read = client.try_clone().map_err(|e| e.to_string())?;
    let upstream_write = upstream.try_clone().map_err(|e| e.to_string())?;
    let upstream_read = upstream.try_clone().map_err(|e| e.to_string())?;

    let a = std::thread::spawn({
        let up = up_bytes.clone();
        let stop = stop.clone();
        let sni = sni_slot.clone();
        move || relay(client_read, upstream_write, up, stop, Some(sni))
    });
    let b = std::thread::spawn({
        let down = down_bytes.clone();
        let stop = stop.clone();
        move || relay(upstream_read, client, down, stop, None)
    });
    let _ = a.join();
    let _ = b.join();

    let sni = sni_slot.lock().ok().and_then(|v| v.clone()).unwrap_or_default();
    let mut request = make_request(offset_secs, started);
    request.method = "CONNECT".to_string();
    request.url = format!("https://{host}/");
    request.host = host.clone();
    request.path = "/".to_string();
    request.scheme = "https".to_string();
    request.status_text = if sni.is_empty() {
        "HTTPS 隧道（未解密）".to_string()
    } else {
        format!("HTTPS 隧道 · SNI {sni}")
    };
    request.mime_type = "application/octet-stream".to_string();
    request.duration_ms = ((now_millis() - started).max(0.0) * 1000.0) as u32;
    request.request_size = up_bytes.load(Ordering::Relaxed);
    request.response_size = down_bytes.load(Ordering::Relaxed);
    request.request_headers = header_pairs(head_lines);
    redact(&mut request);
    push_request(shared, request);
    Ok(())
}

fn handle_connection(
    mut client: TcpStream,
    shared: Arc<Mutex<ProxyBuffer>>,
    stop: Arc<AtomicBool>,
    offset_secs: i32,
) {
    let head = match read_head(&mut client) {
        Ok(head) => head,
        Err(_) => return, // half-open / health check: no evidence to record
    };
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("").to_string();
    let rest: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
    if request_line.is_empty() {
        return;
    }
    let result = if request_line.starts_with("CONNECT ") {
        handle_connect(client, &request_line, &rest, &shared, stop, offset_secs)
    } else {
        handle_http(client, &request_line, &rest, &shared, offset_secs)
    };
    if let Err(reason) = result {
        // A failed connection is evidence, not noise: "the app tried to reach X and the
        // DNS/TLS/connect step failed" is often the whole bug. Record it as a row.
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let target = parts.next().unwrap_or("").to_string();
        let (scheme, host, path) = if target.contains("://") {
            split_url(&target)
        } else {
            ("http".to_string(), target.clone(), target.clone())
        };
        let mut request = make_request(offset_secs, now_millis());
        request.method = method;
        request.url = target;
        request.host = host;
        request.path = path;
        request.scheme = scheme;
        request.error = reason;
        redact(&mut request);
        push_request(&shared, request);
    }
}

/// Translate a bound listener into the endpoint the device must be told about.
/// With `port = 0` the OS picks a free port, so using the *requested* port would point
/// the device at `:0` and black-hole every request.
fn bound_endpoint(listener: &TcpListener, lan: Ipv4Addr) -> Result<(u16, String), String> {
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取监听地址失败: {e}"))?
        .port();
    Ok((port, format!("{lan}:{port}")))
}

pub struct ProxyCapture {
    session_id: String,
    serial: String,
    port: u16,
    info: ProxyCaptureInfo,
    shared: Arc<Mutex<ProxyBuffer>>,
    stop: Arc<AtomicBool>,
    guard: Option<ProxyGuard>,
    thread: Option<JoinHandle<()>>,
}

impl ProxyCapture {
    /// Start listening, then point the device at this machine. Ordering matters: setting
    /// the proxy before the listener exists would black-hole the device's traffic.
    pub fn start(
        adb_path: &Path,
        serial: &str,
        port: u16,
        offset_secs: i32,
        session_id: String,
    ) -> Result<Self, String> {
        let device = device_ip(adb_path, serial);
        let lan = local_lan_ip(device).ok_or_else(|| "无法确定本机局域网地址".to_string())?;
        let listener = TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(lan, port)))
            .map_err(|e| format!("无法监听 {lan}:{port}: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("设置非阻塞失败: {e}"))?;

        let (bound_port, endpoint) = bound_endpoint(&listener, lan)?;
        let guard = ProxyGuard::apply(adb_path, serial, &endpoint)?;
        let shared = Arc::new(Mutex::new(ProxyBuffer::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let allowed = device.map(|d| d.to_string()).unwrap_or_default();
        let allowed_for_thread = allowed.clone();
        let local_addr = lan.to_string();
        let thread = std::thread::spawn(move || {
            let allowed = allowed_for_thread.as_str();
            let local_addr = local_addr.as_str();
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        let source = match peer.ip() {
                            std::net::IpAddr::V4(v4) => v4.to_string(),
                            std::net::IpAddr::V6(v6) => v6.to_string(),
                        };
                        // Only the paired device (and this machine) may use the proxy.
                        // It binds to a LAN address, so without this check anyone on the
                        // same network could route traffic through the desktop app.
                        if !allowed.is_empty()
                            && source != allowed
                            && source != local_addr
                            && source != "127.0.0.1"
                            && source != "::1"
                        {
                            drop(stream);
                            continue;
                        }
                        let shared = Arc::clone(&thread_shared);
                        let stop = Arc::clone(&thread_stop);
                        std::thread::spawn(move || {
                            handle_connection(stream, shared, stop, offset_secs)
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    Err(e) => {
                        if let Ok(mut buffer) = thread_shared.lock() {
                            buffer.error = format!("代理监听异常: {e}");
                            buffer.finished = true;
                        }
                        return;
                    }
                }
            }
            if let Ok(mut buffer) = thread_shared.lock() {
                buffer.finished = true;
            }
        });

        Ok(Self {
            session_id,
            serial: serial.to_string(),
            port: bound_port,
            info: ProxyCaptureInfo {
                endpoint,
                device_ip: device.map(|d| d.to_string()).unwrap_or_default(),
                allowed_sources: if allowed.is_empty() {
                    "未识别设备 IP：仅本机回环可用".to_string()
                } else {
                    format!("{allowed} + 127.0.0.1")
                },
            },
            shared,
            stop,
            guard: Some(guard),
            thread: Some(thread),
        })
    }

    pub fn info(&self) -> ProxyCaptureInfo {
        self.info.clone()
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
            events: buffer.connections,
            target: self.info.endpoint.clone(),
            error: buffer.error.clone(),
        };
        (pending, status)
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(mut guard) = self.guard.take() {
            if let Err(e) = guard.restore() {
                eprintln!("恢复设备代理失败: {e}");
            }
        }
    }
}

impl Drop for ProxyCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // ProxyGuard::drop restores the original proxy even if stop() was never called.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_request_port_reports_the_actual_bound_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let (port, endpoint) = bound_endpoint(&listener, Ipv4Addr::LOCALHOST).unwrap();
        assert_ne!(port, 0, "系统应分配一个真实端口");
        assert_eq!(
            endpoint.parse::<SocketAddr>().unwrap(),
            listener.local_addr().unwrap(),
            "设备必须被告知真正监听的端口，而不是 :0"
        );
    }

    /// The ignored tests below temporarily own the device-wide proxy. If the desktop
    /// app is already capturing (or the device has a proxy for any other reason),
    /// taking it over would black-hole the user's live traffic: skip instead.
    fn device_proxy_is_free(adb_path: &Path, serial: &str) -> bool {
        let script = crate::netprobe::proxy_settings_script();
        let current = adb::shell_probe(adb_path, serial, &script);
        if crate::netprobe::proxy_from_settings(&current).is_empty() {
            return true;
        }
        eprintln!("跳过：设备已有全局代理（可能有正在进行的采集），本测试不覆盖设备设置");
        false
    }

    /// A minimal TLS ClientHello carrying `api.example.com` as SNI.
    fn client_hello(host: &str) -> Vec<u8> {
        let mut name = vec![0u8, 0, 0];
        name[1..3].copy_from_slice(&(host.len() as u16).to_be_bytes());
        name.extend_from_slice(host.as_bytes());
        let mut list = vec![0u8, 0];
        list[0..2].copy_from_slice(&(name.len() as u16).to_be_bytes());
        list.extend_from_slice(&name);
        let mut ext = vec![0u8, 0, 0, 0];
        ext[2..4].copy_from_slice(&(list.len() as u16).to_be_bytes());
        ext.extend_from_slice(&list);

        let mut body = vec![0x03, 0x03];
        body.extend_from_slice(&[0u8; 32]); // random
        body.push(0); // session id length
        body.extend_from_slice(&[0x00, 0x02, 0x13, 0x01]); // one cipher suite
        body.extend_from_slice(&[0x01, 0x00]); // one compression method
        body.extend_from_slice(&(ext.len() as u16).to_be_bytes());
        body.extend_from_slice(&ext);

        let mut handshake = vec![0x01, 0, 0, 0];
        handshake[1..4].copy_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        handshake.extend_from_slice(&body);
        let mut record = vec![0x16, 0x03, 0x01, 0, 0];
        record[3..5].copy_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);
        record
    }

    #[test]
    fn extracts_sni_from_a_client_hello() {
        let hello = client_hello("api.example.com");
        assert_eq!(parse_sni(&hello).as_deref(), Some("api.example.com"));
    }

    #[test]
    fn sni_parser_rejects_non_tls_and_truncated_data() {
        assert_eq!(parse_sni(b"GET / HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_sni(&[0x16, 0x03, 0x01]), None);
        let mut hello = client_hello("x.cn");
        hello.truncate(50);
        assert_eq!(parse_sni(&hello), None);
    }

    #[test]
    fn parses_device_ip_from_ip_addr_output() {
        let text = "    inet 172.21.32.224/22 brd 172.21.35.255 scope global wlan0\n";
        assert_eq!(parse_device_ip(text), Some(Ipv4Addr::new(172, 21, 32, 224)));
        assert_eq!(parse_device_ip("    inet 127.0.0.1/8 scope host lo\n"), None);
    }

    #[test]
    fn lan_ip_is_a_real_non_loopback_address() {
        let ip = local_lan_ip(Some(Ipv4Addr::new(172, 21, 32, 224))).expect("应有本机 LAN 地址");
        assert!(!ip.is_loopback(), "不能把设备代理指向 127.0.0.1");
    }

    #[test]
    fn header_helpers_ignore_case_and_proxy_headers() {
        let headers = header_pairs(&["Host: api.example.com", "Authorization: Bearer x", "Content-Length: 3"]);
        assert_eq!(header_value(&headers, "HOST"), "api.example.com");
        assert_eq!(header_value(&headers, "content-length"), "3");
        assert_eq!(headers.len(), 3);
    }

    #[test]
    fn a_plain_http_request_is_forwarded_and_recorded() {
        // Spin up a tiny upstream HTTP server, then drive `handle_http` with a client
        // socket. This is the core path for cleartext traffic, end to end.
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = upstream.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = upstream.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("POST /v1/homework HTTP/1.1"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"code\":0}")
                .unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_addr = listener.local_addr().unwrap();
        let shared = Arc::new(Mutex::new(ProxyBuffer::default()));
        let handle = std::thread::spawn({
            let shared = Arc::clone(&shared);
            move || {
                let (client, _) = listener.accept().unwrap();
                let line = format!("POST http://127.0.0.1:{}/v1/homework HTTP/1.1", addr.port());
                handle_http(client, &line, &["Content-Type: application/json"], &shared, 0).unwrap();
            }
        });
        let mut client = TcpStream::connect(client_addr).unwrap();
        handle.join().unwrap();
        server.join().unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        assert!(String::from_utf8_lossy(&response).contains("200 OK"));

        let rows = &shared.lock().unwrap().pending;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].method, "POST");
        assert_eq!(rows[0].status, 200);
        assert_eq!(rows[0].path, "/v1/homework");
        assert_eq!(rows[0].mime_type, "application/json");
        assert_eq!(rows[0].source, "proxy");
    }

    // This is the only ignored proxy test on purpose: the device has a single global
    // proxy, so two such tests running in parallel would fight over it and make the
    // read-back verification fail. A deterministic local request replaces the old
    // 6-second "wait for whatever traffic the device happens to send" test.
    #[test]
    #[ignore = "需要连接设备并临时独占其全局代理设置"]
    fn native_proxy_forwards_a_request_and_restores_every_key() {
        let adb_path = adb::locate().expect("应找到 adb");
        let serial = adb::devices()
            .expect("adb devices 应成功")
            .into_iter()
            .find(|d| d.is_ready())
            .expect("需要已授权设备")
            .serial;
        if !device_proxy_is_free(&adb_path, &serial) {
            return;
        }
        let script = crate::netprobe::proxy_settings_script();
        let before = crate::netprobe::parse_proxy_settings(&adb::shell_probe(
            &adb_path,
            &serial,
            &script,
        ));

        // A local upstream keeps the test offline and deterministic.
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = upstream.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).contains("/native-probe"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });

        let mut capture = ProxyCapture::start(&adb_path, &serial, 0, 0, "test".into())
            .expect("应能启动原生接口采集");
        let endpoint = capture.info().endpoint.clone();
        let mut client = TcpStream::connect(&endpoint).expect("应能连上内建代理");
        let request = format!(
            "GET http://127.0.0.1:{upstream_port}/native-probe HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nConnection: close\r\n\r\n"
        );
        client.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        let _ = client.read_to_end(&mut response);
        assert!(String::from_utf8_lossy(&response).contains("200 OK"), "客户端应收到真实响应");
        server.join().unwrap();

        let mut rows = Vec::new();
        for _ in 0..20 {
            let (drained, _) = capture.drain();
            rows.extend(drained);
            if !rows.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        capture.stop();
        let after = crate::netprobe::parse_proxy_settings(&adb::shell_probe(
            &adb_path,
            &serial,
            &script,
        ));
        assert_eq!(before, after, "AOSP 拆出的 host/port 也必须被清掉");
        assert!(
            rows.iter().any(|r| r.method == "GET"
                && r.path == "/native-probe"
                && r.status == 200
                && r.source == "proxy"),
            "代理应记录到明文 HTTP 请求: {rows:?}"
        );
        eprintln!("记录到 {} 条请求，设备代理已完整还原", rows.len());
    }
}
