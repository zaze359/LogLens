//! In-memory log sessions.
//!
//! A file is parsed once into `Vec<LogEntry>`; every subsequent filter only scans this
//! vector. The matched index list is cached by the serialised filter so paging through a
//! large result set does not re-evaluate the rules.

use crate::logcat::{civil_from_days, days_from_civil};
use crate::model::{FilterSpec, LogEntry, NameCount, QueryResult, SearchHits, TimeTimeline};
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
    /// 整份日志的时间轴（绝对毫秒）。只在首次需要时扫描一遍，追加实时日志后失效。
    time_axis: Option<TimeAxis>,
}

impl LogSession {
    // FileInfo is returned to the frontend by `open_log_file` and kept in the Vue store;
    // the session itself only needs the parsed entries plus the match cache.
    pub fn new(entries: Vec<LogEntry>) -> Self {
        Self {
            entries,
            cache: HashMap::new(),
            time_axis: None,
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
        // 新条目可能早于或晚于现有首尾（`adb logcat` 会先回放缓冲区），重新算比扩展更稳。
        self.time_axis = None;
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

    /// 时间轴分布：桶只统计命中条目，但轴的范围取自整份日志。
    ///
    /// 轴取整份日志而不是当前结果集，是为了让轴在筛选时保持稳定 —— 否则换一次筛选轴就
    /// 伸缩一次，区间拖拽的位置会跟着漂。`time_from` / `time_to` 在统计前被摘掉，选中
    /// 一段区间不会让其他时间段从图上消失。
    pub fn timeline(&mut self, filter: &FilterSpec, bins: u32) -> Result<TimeTimeline, String> {
        let axis = match self.time_axis {
            Some(axis) => axis,
            None => match compute_time_axis(&self.entries) {
                Some(axis) => {
                    self.time_axis = Some(axis);
                    axis
                }
                // 整份日志都没有可解析的时间戳：没有轴可画。
                None => return Ok(TimeTimeline::default()),
            },
        };

        let mut base = filter.clone();
        base.time_from.clear();
        base.time_to.clear();
        let compiled = CompiledFilter::compile(&base)?;
        let indices = self.matched_indices(&base, &compiled);

        let bins = bins.clamp(1, 512) as usize;
        // 所有条目落在同一毫秒时给一个最小跨度，避免除零。
        let span = (axis.end - axis.start).max(1);
        let mut counts = vec![0u32; bins];
        let mut total: u32 = 0;
        let mut untimed: u32 = 0;
        for &i in &indices {
            let Some(entry) = self.entries.get(i as usize) else {
                continue;
            };
            match entry_time_millis(entry, axis.has_date) {
                Some(ms) => {
                    let bucket = (((ms - axis.start) as u128 * bins as u128) / span as u128)
                        .min(bins as u128 - 1) as usize;
                    counts[bucket] += 1;
                    total += 1;
                }
                None => untimed += 1,
            }
        }

        Ok(TimeTimeline {
            start: axis_label(axis.start, &axis),
            end: axis_label(axis.end, &axis),
            start_ms: axis.start,
            end_ms: axis.end,
            has_date: axis.has_date,
            has_year: axis.has_year,
            bins: counts,
            total,
            untimed,
        })
    }
}

/// 时间轴的跨度与日期信息。轴一旦算好就缓存 —— 每次筛选重扫整份日志没有意义。
#[derive(Clone, Copy)]
struct TimeAxis {
    /// 轴起点，1970-01-01 起的绝对毫秒。
    start: u64,
    end: u64,
    /// 日志是否带日期；`false` 时轴只有时刻，`start` / `end` 只是当日毫秒。
    has_date: bool,
    /// 日期是否带真实年份（`MM-DD` 形式的日志用 1970 占位）。
    has_year: bool,
}

/// `MM-DD` 这类没有年份的日志使用的占位年份，展示时会去掉。
const YEAR_PLACEHOLDER: i64 = 1970;

/// 取条目在轴上的毫秒值：有日期的文件按完整日期，否则退回当日时刻。
fn entry_time_millis(entry: &LogEntry, axis_has_date: bool) -> Option<u64> {
    if axis_has_date {
        datetime_key_millis(&entry.datetime_key).map(|(ms, _)| ms)
    } else {
        time_key_millis(&entry.time_key)
    }
}

/// `YYYY-MM-DD HH:MM:SS.mmm` → 1970-01-01 起的毫秒，附带年份是否真实。
fn datetime_key_millis(key: &str) -> Option<(u64, bool)> {
    let (date, time) = key.trim().split_once(' ')?;
    let tod = time_key_millis(time)?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    if days < 0 {
        return None;
    }
    Some((days as u64 * 86_400_000 + tod, y != YEAR_PLACEHOLDER))
}

/// 轴端点标签：带年份 `YYYY-MM-DD HH:MM:SS.mmm`，只有月日 `MM-DD HH:MM:SS.mmm`。
fn axis_label(ms: u64, axis: &TimeAxis) -> String {
    let tod = ms % 86_400_000;
    let time = millis_to_time_key(tod);
    if !axis.has_date {
        return time;
    }
    let (y, m, d) = civil_from_days((ms / 86_400_000) as i64);
    if axis.has_year {
        format!("{y:04}-{m:02}-{d:02} {time}")
    } else {
        format!("{m:02}-{d:02} {time}")
    }
}

/// `HH:MM:SS.mmm` → 当日毫秒；格式不符返回 `None`（日志时间戳不带日期，不猜跨天）。
fn time_key_millis(key: &str) -> Option<u64> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let (base, frac) = match key.split_once('.') {
        Some((base, frac)) => (base, frac),
        None => (key, ""),
    };
    let mut parts = base.split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let s: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || h > 23 || m > 59 || s > 59 {
        return None;
    }
    let mut millis: String = frac.chars().take(3).collect();
    while millis.len() < 3 {
        millis.push('0');
    }
    let ms: u64 = millis.parse().ok()?;
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

fn millis_to_time_key(value: u64) -> String {
    let h = value / 3_600_000;
    let m = (value % 3_600_000) / 60_000;
    let s = (value % 60_000) / 1000;
    let ms = value % 1000;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

fn compute_time_axis(entries: &[LogEntry]) -> Option<TimeAxis> {
    // 有日期的文件按完整日期建轴；整份日志都没有日期时退回当日时刻。
    let has_date = entries.iter().any(|entry| !entry.datetime_key.is_empty());
    let mut start = u64::MAX;
    let mut end = 0u64;
    let mut has_year = false;
    let mut found = false;
    for entry in entries {
        let value = if has_date {
            datetime_key_millis(&entry.datetime_key)
        } else {
            time_key_millis(&entry.time_key).map(|ms| (ms, false))
        };
        if let Some((ms, year)) = value {
            found = true;
            has_year |= year;
            start = start.min(ms);
            end = end.max(ms);
        }
    }
    found.then_some(TimeAxis {
        start,
        end,
        has_date,
        has_year,
    })
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
    /// One proxy guard per device. Dropping the guard clears the device proxy.
    pub proxy_guards: Mutex<HashMap<String, ProxyGuard>>,
    /// Serials whose proxy capture is *currently starting*. The duplicate check and the
    /// actual start are separated by several `adb` round trips, so without a reservation
    /// two near-simultaneous starts would both install a guard for the same device.
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
                crate::error_log::record_flow("proxy.restore", "退出时清空设备代理失败", &e);
            }
        }
        let mut guards = self.proxy_guards.lock().unwrap_or_else(|p| p.into_inner());
        guards.clear(); // ProxyGuard::drop clears the device proxy
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

    fn timed(time_key: &str, level: &str) -> LogEntry {
        LogEntry {
            time_key: time_key.to_string(),
            level: level.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn timeline_axis_covers_the_whole_file_regardless_of_row_order() {
        let mut s = LogSession::new(vec![
            timed("10:00:10.000", "I"),
            timed("10:00:00.000", "I"),
            timed("10:00:05.000", "I"),
        ]);
        let timeline = s.timeline(&FilterSpec::default(), 5).expect("时间轴应成功");
        assert_eq!(timeline.start, "10:00:00.000");
        assert_eq!(timeline.end, "10:00:10.000");
        assert_eq!(timeline.bins.iter().sum::<u32>(), 3);
        assert_eq!(timeline.total, 3);
        assert_eq!(timeline.untimed, 0);
    }

    #[test]
    fn timeline_ignores_the_selected_range_so_other_periods_stay_visible() {
        let mut s = LogSession::new(vec![
            timed("10:00:00.000", "I"),
            timed("10:00:05.000", "I"),
            timed("10:00:10.000", "I"),
        ]);
        // 只看最后一条：结果集收窄到 1 条，但轴上的分布仍应是三条全貌。
        let filter = FilterSpec {
            time_from: "10:00:06".to_string(),
            time_to: "10:00:12".to_string(),
            ..Default::default()
        };
        assert_eq!(s.query(&filter, 0, 50).expect("查询应成功").matched, 1);
        let timeline = s.timeline(&filter, 4).expect("时间轴应成功");
        assert_eq!(timeline.total, 3, "时间轴必须忽略自己的区间条件");
        assert_eq!(timeline.start, "10:00:00.000");
        assert_eq!(timeline.end, "10:00:10.000");
    }

    #[test]
    fn timeline_counts_entries_without_a_time_key_separately() {
        let mut s = LogSession::new(vec![timed("10:00:00.000", "I"), timed("", "?")]);
        let timeline = s.timeline(&FilterSpec::default(), 2).expect("时间轴应成功");
        assert_eq!(timeline.total, 1);
        assert_eq!(timeline.untimed, 1, "读不出时间戳的行不该悄悄丢掉");
        assert_eq!(timeline.bins.iter().sum::<u32>(), 1);
    }

    #[test]
    fn timeline_still_applies_other_filters() {
        let mut s = LogSession::new(vec![
            timed("10:00:00.000", "E"),
            timed("10:00:01.000", "I"),
            timed("10:00:02.000", "E"),
        ]);
        let filter = FilterSpec {
            levels: vec!["E".to_string()],
            ..Default::default()
        };
        let timeline = s.timeline(&filter, 3).expect("时间轴应成功");
        assert_eq!(timeline.total, 2, "级别筛选必须作用于时间轴");
    }

    #[test]
    fn timeline_is_empty_when_no_entry_has_a_timestamp() {
        let mut s = LogSession::new(vec![entry("no timestamp")]);
        let timeline = s.timeline(&FilterSpec::default(), 4).expect("时间轴应成功");
        assert!(timeline.bins.is_empty());
        assert_eq!(timeline.total, 0);
    }

    fn dated(datetime_key: &str, time_key: &str) -> LogEntry {
        LogEntry {
            datetime_key: datetime_key.to_string(),
            time_key: time_key.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn timeline_axis_includes_dates_when_the_log_has_them() {
        let mut s = LogSession::new(vec![
            dated("2026-11-12 10:00:00.000", "10:00:00.000"),
            dated("2026-11-12 12:30:00.000", "12:30:00.000"),
        ]);
        let timeline = s.timeline(&FilterSpec::default(), 2).expect("时间轴应成功");
        assert!(timeline.has_date);
        assert!(timeline.has_year);
        assert_eq!(timeline.start, "2026-11-12 10:00:00.000");
        assert_eq!(timeline.end, "2026-11-12 12:30:00.000");
        assert!(timeline.start_ms < timeline.end_ms);
    }

    #[test]
    fn timeline_axis_crosses_midnight() {
        let mut s = LogSession::new(vec![
            dated("2026-11-12 23:50:00.000", "23:50:00.000"),
            dated("2026-11-13 00:10:00.000", "00:10:00.000"),
        ]);
        let timeline = s.timeline(&FilterSpec::default(), 4).expect("时间轴应成功");
        assert_eq!(timeline.start, "2026-11-12 23:50:00.000");
        assert_eq!(timeline.end, "2026-11-13 00:10:00.000");
        assert_eq!(timeline.bins.iter().sum::<u32>(), 2, "跨天的两条都要落在轴上");
    }

    #[test]
    fn timeline_strips_the_placeholder_year_from_labels() {
        let mut s = LogSession::new(vec![dated("1970-11-12 10:00:00.000", "10:00:00.000")]);
        let timeline = s.timeline(&FilterSpec::default(), 2).expect("时间轴应成功");
        assert!(timeline.has_date);
        assert!(!timeline.has_year, "占位年份不能被当成真实年份");
        assert_eq!(timeline.start, "11-12 10:00:00.000");
    }
}
