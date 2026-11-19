//! In-memory log sessions.
//!
//! A file is parsed once into `Vec<LogEntry>`; every subsequent filter only scans this
//! vector. The matched index list is cached by the serialised filter so paging through a
//! large result set does not re-evaluate the rules.

use crate::model::{FilterSpec, LogEntry, NameCount, QueryResult, SearchHits};
use crate::netcapture::{AnyCapture, ProxyGuard};
use crate::network::NetworkRequest;
use crate::proxy::ProxyRuntime;
use crate::rules::{compute_matches, CompiledFilter, TextMatcher};
use crate::stream::DeviceStream;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MAX_CACHED_QUERIES: usize = 12;

pub struct LogSession {
    pub entries: Vec<LogEntry>,
    cache: HashMap<String, Vec<u32>>,
}

impl LogSession {
    // FileInfo is returned to the frontend by `open_log_file` and kept in the Vue store;
    // the session itself only needs the parsed entries plus the match cache.
    pub fn new(entries: Vec<LogEntry>) -> Self {
        Self {
            entries,
            cache: HashMap::new(),
        }
    }

    /// Append entries produced by a live capture. Indices are assigned here because only
    /// the session knows the current offset; the match cache is invalidated because every
    /// cached index list was computed against a shorter entry vector.
    pub fn append_all(&mut self, pending: Vec<LogEntry>) -> u32 {
        if pending.is_empty() {
            return 0;
        }
        let base = self.entries.len() as u32;
        self.entries.reserve(pending.len());
        for (i, mut entry) in pending.into_iter().enumerate() {
            entry.index = base + i as u32;
            self.entries.push(entry);
        }
        self.cache.clear();
        self.entries.len() as u32
    }

    pub fn matched_indices(&mut self, filter: &FilterSpec, compiled: &CompiledFilter) -> Vec<u32> {
        // `order` only affects paging, not the matched set, so keep it out of the key.
        let mut key_filter = filter.clone();
        key_filter.order.clear();
        // In `find` mode the term never participates in matching, so leaving it in the key
        // would add a fresh cached match list for every keystroke.
        if key_filter.search_mode == "find" {
            key_filter.search.clear();
        }
        let key = serde_json::to_string(&key_filter).unwrap_or_default();
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let indices = compute_matches(&self.entries, compiled);
        if self.cache.len() >= MAX_CACHED_QUERIES {
            self.cache.clear();
        }
        self.cache.insert(key, indices.clone());
        indices
    }

    /// Where `filter.search` appears inside the entries the filter already matched.
    ///
    /// `find` must not change the result set, so this reports positions and never feeds the
    /// table. The cache key is built from a copy with the term cleared, so every keystroke
    /// does not add a new cached match list. It scans `message` (what the table renders, stack
    /// lines included) rather than `raw`: a hit inside the timestamp or tag prefix could not
    /// be highlighted in the row, and "27 hits, none visible" is worse than not matching.
    pub fn find(&mut self, filter: &FilterSpec, limit: usize) -> Result<SearchHits, String> {
        let matcher = TextMatcher::new(&filter.search, filter.is_regex, filter.case_sensitive)?
            .ok_or_else(|| "请先输入查找关键字".to_string())?;
        let compiled = CompiledFilter::compile(filter)?;
        let mut indices = self.matched_indices(filter, &compiled);

        // Hit list is documented as "display order". `matched_indices` is always file order,
        // so a descending view has to flip it — otherwise "next" walks up the screen while
        // the counter still reads 3/27. Reversing the clone leaves the cached list untouched.
        if filter.order.eq_ignore_ascii_case("desc") {
            indices.reverse();
        }

        let mut hits: Vec<u32> = Vec::new();
        let mut total: u32 = 0;
        for index in indices {
            let Some(entry) = self.entries.get(index as usize) else {
                continue;
            };
            if matcher.is_match(&entry.message) {
                total += 1;
                if hits.len() < limit {
                    hits.push(index);
                }
            }
        }
        Ok(SearchHits {
            total,
            truncated: (hits.len() as u32) < total,
            indices: hits,
        })
    }

    pub fn query(
        &mut self,
        filter: &FilterSpec,
        offset: u32,
        limit: u32,
    ) -> Result<QueryResult, String> {
        let started = Instant::now();
        let compiled = CompiledFilter::compile(filter)?;
        let indices = self.matched_indices(filter, &compiled);
        let matched = indices.len() as u32;

        let mut level_map: HashMap<String, u32> = HashMap::new();
        let mut tag_map: HashMap<String, u32> = HashMap::new();
        for &i in &indices {
            if let Some(e) = self.entries.get(i as usize) {
                *level_map.entry(e.level.clone()).or_insert(0) += 1;
                if !e.tag.is_empty() {
                    *tag_map.entry(e.tag.clone()).or_insert(0) += 1;
                }
            }
        }
        let level_counts: Vec<NameCount> = ["V", "D", "I", "W", "E", "F", "A", "S", "?"]
            .iter()
            .filter_map(|level| {
                level_map
                    .get(*level)
                    .map(|&count| NameCount { name: (*level).to_string(), count })
            })
            .collect();
        let mut top_tags: Vec<NameCount> = tag_map
            .into_iter()
            .map(|(name, count)| NameCount { name, count })
            .collect();
        top_tags.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        top_tags.truncate(10);

        let descending = filter.order.eq_ignore_ascii_case("desc");
        let page: Vec<LogEntry> = if descending {
            indices
                .iter()
                .rev()
                .skip(offset as usize)
                .take(limit as usize)
                .filter_map(|&i| self.entries.get(i as usize).cloned())
                .collect()
        } else {
            indices
                .iter()
                .skip(offset as usize)
                .take(limit as usize)
                .filter_map(|&i| self.entries.get(i as usize).cloned())
                .collect()
        };

        Ok(QueryResult {
            total: self.entries.len() as u32,
            matched,
            entries: page,
            level_counts,
            top_tags,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// A collected set of network requests, keyed by capture id. Kept out of `LogSession`
/// because the columns are disjoint (method/status/duration vs tag/level/pid).
pub struct NetworkSession {
    pub requests: Vec<NetworkRequest>,
    /// 原生采集识别到的设备 IP；空表示没读到（流量会落到独立代理页）。
    pub device_ip: String,
    pub label: String,
    pub source: String,
    pub device: String,
    pub streaming: bool,
}

pub struct AppState {
    pub sessions: Mutex<HashMap<String, LogSession>>,
    /// Live captures, keyed by the session they append to.
    pub streams: Mutex<HashMap<String, DeviceStream>>,
    /// Live CDP captures and their collected requests.
    pub network_captures: Mutex<HashMap<String, AnyCapture>>,
    pub network_sessions: Mutex<HashMap<String, NetworkSession>>,
    /// One proxy guard per device. Dropping the guard restores the previous setting.
    pub proxy_guards: Mutex<HashMap<String, ProxyGuard>>,
    /// Serials whose proxy capture is *currently starting*. The duplicate check and the
    /// actual start are separated by several `adb` round trips, so without a reservation
    /// two near-simultaneous starts would both install a guard — and the second would
    /// remember the first's endpoint as "the original setting" to restore.
    starting_proxies: Mutex<HashSet<String>>,
    /// The one proxy listener. Devices are routes on it and the standalone page is its
    /// unnamed-source view — a single runtime is what keeps the endpoint stable at 18999
    /// instead of one listener per consumer racing for the port.
    pub proxy_runtime: Arc<Mutex<ProxyRuntime>>,
    /// Abort flags for in-flight file imports, keyed by the frontend's request id. The flag
    /// is shared with the parsing thread because a cancelled import cannot use the session
    /// lock (it has no session yet).
    pub import_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    seq: AtomicU64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::new()),
            network_captures: Mutex::new(HashMap::new()),
            network_sessions: Mutex::new(HashMap::new()),
            proxy_guards: Mutex::new(HashMap::new()),
            starting_proxies: Mutex::new(HashSet::new()),
            proxy_runtime: Arc::new(Mutex::new(ProxyRuntime::new())),
            import_cancels: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
        }
    }
}

impl AppState {
    pub fn next_id(&self) -> String {
        format!("s{}", self.seq.fetch_add(1, Ordering::Relaxed))
    }

    /// Claim the right to start a proxy capture for `serial`. `None` means another start
    /// for the same device is already in flight. The returned guard releases the claim on
    /// drop (success *or* failure), so a failed start never locks the device out.
    pub fn begin_proxy_start(&self, serial: &str) -> Option<ProxyStartGuard<'_>> {
        let mut starting = self
            .starting_proxies
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !starting.insert(serial.to_string()) {
            return None;
        }
        Some(ProxyStartGuard {
            state: self,
            serial: serial.to_string(),
        })
    }

    /// Kill every live capture and undo every device-side change. Called when the window
    /// is destroyed: an `adb logcat` child, a CDP forward or a lingering global proxy
    /// must not outlive the app — the last one would leave the phone offline.
    pub fn stop_all_streams(&self) {
        // A poisoned lock must NOT stop cleanup: leaving the device pointing at a
        // dead proxy is far worse than a panicked worker thread.
        let mut streams = self.streams.lock().unwrap_or_else(|p| p.into_inner());
        let mut stopped_streams = 0u32;
        for (_, mut stream) in streams.drain() {
            stream.stop();
            stopped_streams += 1;
        }
        let mut captures = self
            .network_captures
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut stopped_captures = 0u32;
        for (_, mut capture) in captures.drain() {
            stopped_captures += 1;
            // The device may already be gone; the failure is logged instead of ignored,
            // and the journal entry is kept for `heal_stale_proxies` / the retry thread.
            if let Err(e) = capture.stop() {
                crate::error_log::record_flow("proxy.restore", "退出时恢复设备代理失败", &e);
            }
        }
        let mut guards = self.proxy_guards.lock().unwrap_or_else(|p| p.into_inner());
        guards.clear(); // ProxyGuard::drop restores the original proxy
        // The proxy listener outlives every capture while the app runs, so it has to be
        // closed here explicitly or the port would outlive the window.
        let proxy_closed = {
            let mut runtime = self.proxy_runtime.lock().unwrap_or_else(|p| p.into_inner());
            let was_running = runtime.running();
            runtime.shutdown();
            was_running
        };
        crate::error_log::record_flow(
            "app.shutdown",
            "已停止所有实时采集",
            &format!(
                "streams={stopped_streams} captures={stopped_captures} proxyClosed={proxy_closed}"
            ),
        );
    }
}

/// Releases a [`AppState::begin_proxy_start`] claim. Dropping it is the only way to free
/// the device for the next start, which is why it is returned instead of a bool.
pub struct ProxyStartGuard<'a> {
    state: &'a AppState,
    serial: String,
}

impl Drop for ProxyStartGuard<'_> {
    fn drop(&mut self) {
        let mut starting = self
            .state
            .starting_proxies
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        starting.remove(&self.serial);
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        // Last line of defence for a graceful shutdown that bypassed the window
        // events. A kill -9 still skips this; `heal_stale_proxies()` on the next
        // launch is what actually guarantees the device is never left offline.
        self.stop_all_streams();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: &str) -> LogEntry {
        LogEntry {
            message: message.to_string(),
            raw: message.to_string(),
            ..Default::default()
        }
    }

    fn session() -> LogSession {
        LogSession::new(vec![
            entry("alpha one"),
            entry("beta"),
            entry("beta alpha two"),
            entry("gamma"),
        ])
    }

    fn find_filter(term: &str) -> FilterSpec {
        FilterSpec {
            search: term.to_string(),
            search_mode: "find".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn find_reports_positions_without_narrowing_the_list() {
        let mut s = session();
        let filter = find_filter("alpha");

        let hits = s.find(&filter, 100).expect("查找应成功");
        assert_eq!(hits.total, 2);
        assert_eq!(hits.indices, vec![0, 2]);
        assert!(!hits.truncated);

        // The whole point of the split: the same filter still lists every entry.
        let result = s.query(&filter, 0, 50).expect("查询应成功");
        assert_eq!(result.matched, 4, "find 模式不能改变结果集");
        assert_eq!(result.total, 4);
    }

    #[test]
    fn find_follows_display_order_when_descending() {
        let mut s = session();
        let mut filter = find_filter("alpha");
        filter.order = "desc".to_string();
        let hits = s.find(&filter, 100).expect("查找应成功");
        // 显示顺序是倒序：文件里靠后的命中要先出现。
        assert_eq!(hits.indices, vec![2, 0]);
    }

    #[test]
    fn filter_mode_still_narrows() {
        let mut s = session();
        let filter = FilterSpec {
            search: "alpha".to_string(),
            search_mode: "filter".to_string(),
            ..Default::default()
        };
        let result = s.query(&filter, 0, 50).expect("查询应成功");
        assert_eq!(result.matched, 2);
    }

    #[test]
    fn find_caps_the_index_list_but_keeps_the_count() {
        let mut s = session();
        let hits = s.find(&find_filter("beta"), 1).expect("查找应成功");
        assert_eq!(hits.total, 2);
        assert_eq!(hits.indices.len(), 1, "index 列表应被 limit 截断");
        assert!(hits.truncated);
    }

    #[test]
    fn find_scans_the_message_body_including_stack_lines() {
        let mut s = LogSession::new(vec![
            entry("FATAL EXCEPTION: main"),
            entry("nothing here"),
        ]);
        // Stack frames live in `message`, not on the first line; a search has to reach them.
        s.entries[0].message.push_str("\n\tat com.example.Crash.run(Crash.kt:42)");
        let hits = s.find(&find_filter("Crash.kt"), 10).expect("查找应成功");
        assert_eq!(hits.total, 1);
        assert_eq!(hits.indices, vec![0]);
    }

    #[test]
    fn find_rejects_an_empty_term() {
        let mut s = session();
        assert!(s.find(&find_filter(""), 10).is_err());
    }
}
