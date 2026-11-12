//! In-memory log sessions.
//!
//! A file is parsed once into `Vec<LogEntry>`; every subsequent filter only scans this
//! vector. The matched index list is cached by the serialised filter so paging through a
//! large result set does not re-evaluate the rules.

use crate::model::{FilterSpec, LogEntry, NameCount, QueryResult};
use crate::rules::{compute_matches, CompiledFilter};
use crate::stream::DeviceStream;
use std::collections::HashMap;
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

pub struct AppState {
    pub sessions: Mutex<HashMap<String, LogSession>>,
    /// Live captures, keyed by the session they append to.
    pub streams: Mutex<HashMap<String, DeviceStream>>,
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
            import_cancels: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
        }
    }
}

impl AppState {
    pub fn next_id(&self) -> String {
        format!("s{}", self.seq.fetch_add(1, Ordering::Relaxed))
    }

    /// Kill every live capture. Called when the window is destroyed so `adb logcat`
    /// children do not outlive the app.
    pub fn stop_all_streams(&self) {
        if let Ok(mut streams) = self.streams.lock() {
            for (_, mut stream) in streams.drain() {
                stream.stop();
            }
        }
    }
}
