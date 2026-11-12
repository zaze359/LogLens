//! Rule engine.
//!
//! A filter is compiled once per query (regexes included), then evaluated against every
//! cached log entry. Compiling up front keeps the hot loop allocation-free and lets us
//! report invalid regexes to the UI before any work is done.

use crate::logcat::normalize_time_value;
use crate::model::{Condition, FilterSpec, LogEntry, RuleGroup};
use regex::Regex;
use std::borrow::Cow;
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Logic {
    And,
    Or,
}

fn parse_logic(s: &str) -> Logic {
    if s.eq_ignore_ascii_case("OR") {
        Logic::Or
    } else {
        Logic::And
    }
}

fn with_case(pattern: &str, case_sensitive: bool) -> String {
    // Built-in templates already carry `(?i)`; avoid stacking duplicate flags.
    if case_sensitive || pattern.trim().is_empty() || pattern.starts_with("(?i)") {
        pattern.to_string()
    } else {
        format!("(?i){pattern}")
    }
}

fn eq_text(a: &str, b: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        a == b
    } else {
        a.eq_ignore_ascii_case(b)
    }
}

fn contains_text(hay: &str, needle: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        hay.contains(needle)
    } else {
        hay.to_lowercase().contains(&needle.to_lowercase())
    }
}

fn compare_values(a: &str, b: &str) -> Ordering {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => a.cmp(b),
    }
}

struct CompiledCondition {
    field: String,
    op: String,
    value: String,
    negate: bool,
    case_sensitive: bool,
    regex: Option<Regex>,
    list: Vec<String>,
}

impl CompiledCondition {
    fn compile(c: &Condition) -> Result<Self, String> {
        let regex = if c.op == "regex" {
            Some(
                Regex::new(&with_case(&c.value, c.case_sensitive))
                    .map_err(|e| format!("条件正则无效 [{}]: {e}", c.value))?,
            )
        } else {
            None
        };
        let list = c
            .value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(Self {
            field: c.field.clone(),
            op: c.op.clone(),
            value: c.value.clone(),
            negate: c.negate,
            case_sensitive: c.case_sensitive,
            regex,
            list,
        })
    }

    fn field_value<'a>(&self, e: &'a LogEntry) -> Cow<'a, str> {
        match self.field.as_str() {
            "level" => Cow::Borrowed(e.level.as_str()),
            "tag" => Cow::Borrowed(e.tag.as_str()),
            "message" => Cow::Borrowed(e.message.as_str()),
            "raw" => Cow::Borrowed(e.raw.as_str()),
            "package" => Cow::Borrowed(e.package.as_str()),
            "component" => Cow::Borrowed(e.component.as_str()),
            "timestamp" => Cow::Borrowed(e.timestamp.as_str()),
            "timeKey" => Cow::Borrowed(e.time_key.as_str()),
            "pid" => Cow::Owned(e.pid.to_string()),
            "tid" => Cow::Owned(e.tid.to_string()),
            _ => Cow::Borrowed(e.message.as_str()),
        }
    }

    fn matches(&self, e: &LogEntry) -> bool {
        let hay = self.field_value(e);
        let hay = hay.as_ref();
        let cs = self.case_sensitive;
        let ok = match self.op.as_str() {
            "contains" => contains_text(hay, &self.value, cs),
            "notContains" => !contains_text(hay, &self.value, cs),
            "equals" => eq_text(hay, &self.value, cs),
            "notEquals" => !eq_text(hay, &self.value, cs),
            "startsWith" => {
                if cs {
                    hay.starts_with(&self.value)
                } else {
                    hay.to_lowercase().starts_with(&self.value.to_lowercase())
                }
            }
            "endsWith" => {
                if cs {
                    hay.ends_with(&self.value)
                } else {
                    hay.to_lowercase().ends_with(&self.value.to_lowercase())
                }
            }
            "regex" => self.regex.as_ref().is_some_and(|r| r.is_match(hay)),
            "in" => self.list.iter().any(|v| eq_text(hay, v, cs)),
            "gte" => compare_values(hay, &self.value) != Ordering::Less,
            "lte" => compare_values(hay, &self.value) != Ordering::Greater,
            // Unknown operator: treat as pass-through rather than silently hiding data.
            _ => true,
        };
        if self.negate {
            !ok
        } else {
            ok
        }
    }
}

struct CompiledGroup {
    logic: Logic,
    conditions: Vec<CompiledCondition>,
}

impl CompiledGroup {
    fn compile(g: &RuleGroup) -> Result<Self, String> {
        let mut conditions = Vec::with_capacity(g.conditions.len());
        for c in &g.conditions {
            if c.value.trim().is_empty() {
                continue;
            }
            conditions.push(CompiledCondition::compile(c)?);
        }
        Ok(Self {
            logic: parse_logic(&g.logic),
            conditions,
        })
    }

    fn matches(&self, e: &LogEntry) -> bool {
        if self.conditions.is_empty() {
            return true;
        }
        match self.logic {
            Logic::And => self.conditions.iter().all(|c| c.matches(e)),
            Logic::Or => self.conditions.iter().any(|c| c.matches(e)),
        }
    }
}

struct QuickFilter {
    levels: Vec<String>,
    tags: Vec<String>,
    pids: Vec<String>,
    packages: Vec<String>,
    search: String,
    search_regex: Option<Regex>,
    case_sensitive: bool,
    excludes: Vec<String>,
    time_from: String,
    time_to: String,
}

impl QuickFilter {
    fn compile(f: &FilterSpec) -> Result<Self, String> {
        let search_regex = if f.is_regex && !f.search.trim().is_empty() {
            Some(
                Regex::new(&with_case(&f.search, f.case_sensitive))
                    .map_err(|e| format!("搜索正则无效: {e}"))?,
            )
        } else {
            None
        };
        let excludes = f
            .exclude
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(Self {
            levels: f.levels.iter().map(|s| s.to_uppercase()).collect(),
            tags: f.tags.clone(),
            pids: f.pids.clone(),
            packages: f.packages.clone(),
            search: f.search.clone(),
            search_regex,
            case_sensitive: f.case_sensitive,
            excludes,
            time_from: normalize_time_value(&f.time_from),
            time_to: normalize_time_value(&f.time_to),
        })
    }

    fn is_active(&self) -> bool {
        !self.levels.is_empty()
            || !self.tags.is_empty()
            || !self.pids.is_empty()
            || !self.packages.is_empty()
            || !self.search.is_empty()
            || !self.excludes.is_empty()
            || !self.time_from.is_empty()
            || !self.time_to.is_empty()
    }

    fn matches(&self, e: &LogEntry) -> bool {
        if !self.levels.is_empty() && !self.levels.iter().any(|l| l == &e.level) {
            return false;
        }
        if !self.tags.is_empty() && !self.tags.iter().any(|t| t == &e.tag) {
            return false;
        }
        if !self.pids.is_empty() && !self.pids.iter().any(|p| p == &e.pid.to_string()) {
            return false;
        }
        if !self.packages.is_empty() && !self.packages.iter().any(|p| p == &e.package) {
            return false;
        }
        if !self.time_from.is_empty()
            && (e.time_key.is_empty() || e.time_key.as_str() < self.time_from.as_str())
        {
            return false;
        }
        if !self.time_to.is_empty()
            && (e.time_key.is_empty() || e.time_key.as_str() > self.time_to.as_str())
        {
            return false;
        }
        if self.search.is_empty() && self.excludes.is_empty() {
            return true;
        }
        let hay: Cow<'_, str> = if self.case_sensitive {
            Cow::Borrowed(e.raw.as_str())
        } else {
            Cow::Owned(e.raw.to_lowercase())
        };
        let hay = hay.as_ref();
        for keyword in &self.excludes {
            let needle = if self.case_sensitive {
                keyword.clone()
            } else {
                keyword.to_lowercase()
            };
            if hay.contains(&needle) {
                return false;
            }
        }
        if !self.search.is_empty() {
            return match &self.search_regex {
                Some(re) => re.is_match(hay),
                None => {
                    let needle = if self.case_sensitive {
                        self.search.clone()
                    } else {
                        self.search.to_lowercase()
                    };
                    hay.contains(&needle)
                }
            };
        }
        true
    }
}

pub struct CompiledFilter {
    quick: QuickFilter,
    group: Option<CompiledGroup>,
    group_logic: Logic,
}

impl CompiledFilter {
    pub fn compile(f: &FilterSpec) -> Result<Self, String> {
        let group = match &f.group {
            Some(g) if !g.conditions.is_empty() => Some(CompiledGroup::compile(g)?),
            _ => None,
        };
        Ok(Self {
            quick: QuickFilter::compile(f)?,
            group,
            group_logic: parse_logic(&f.group_logic),
        })
    }

    pub fn matches(&self, e: &LogEntry) -> bool {
        let quick_ok = self.quick.matches(e);
        match &self.group {
            None => quick_ok,
            Some(group) => {
                if !self.quick.is_active() {
                    group.matches(e)
                } else if self.group_logic == Logic::Or {
                    quick_ok || group.matches(e)
                } else {
                    quick_ok && group.matches(e)
                }
            }
        }
    }
}

/// Return indices of all matching entries in file order.
pub fn compute_matches(entries: &[LogEntry], filter: &CompiledFilter) -> Vec<u32> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            if filter.matches(e) {
                Some(i as u32)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Condition;

    fn entry(level: &str, tag: &str, message: &str) -> LogEntry {
        LogEntry {
            level: level.to_string(),
            tag: tag.to_string(),
            message: message.to_string(),
            raw: format!("{level}/{tag}: {message}"),
            time_key: "10:23:45.123".to_string(),
            parse_ok: true,
            ..Default::default()
        }
    }

    fn condition(field: &str, op: &str, value: &str) -> Condition {
        Condition {
            field: field.to_string(),
            op: op.to_string(),
            value: value.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn quick_level_and_exclude() {
        let entries = vec![
            entry("D", "OkHttp", "GET https://example.com"),
            entry("E", "AndroidRuntime", "NullPointerException"),
        ];
        let f = FilterSpec {
            levels: vec!["E".to_string()],
            exclude: "https".to_string(),
            ..Default::default()
        };
        let compiled = CompiledFilter::compile(&f).unwrap();
        let hits = compute_matches(&entries, &compiled);
        assert_eq!(hits, vec![1]);
    }

    #[test]
    fn quick_package_filter_matches_any_selected_app() {
        let mut app = entry("D", "OkHttp", "GET /api/paper");
        app.package = "com.hailiang.workcloud.student".to_string();
        let mut other = entry("D", "OkHttp", "GET /api/home");
        other.package = "com.tencent.mm".to_string();
        let unnamed = entry("D", "logd", "audit");
        let entries = vec![app, other, unnamed];

        // Selecting several packages is an OR, matching the tag / pid behaviour.
        let f = FilterSpec {
            packages: vec![
                "com.hailiang.workcloud.student".to_string(),
                "com.tencent.mm".to_string(),
            ],
            ..Default::default()
        };
        let compiled = CompiledFilter::compile(&f).unwrap();
        assert_eq!(compute_matches(&entries, &compiled), vec![0, 1]);

        let single = FilterSpec {
            packages: vec!["com.tencent.mm".to_string()],
            ..Default::default()
        };
        let compiled = CompiledFilter::compile(&single).unwrap();
        assert_eq!(compute_matches(&entries, &compiled), vec![1]);
    }

    #[test]
    fn or_group_with_regex_and_negate() {
        let entries = vec![
            entry("W", "Choreographer", "Skipped 60 frames"),
            entry("I", "MyTag", "normal message"),
        ];
        let f = FilterSpec {
            group: Some(RuleGroup {
                logic: "OR".to_string(),
                conditions: vec![condition("message", "regex", r"Skipped \d+ frames")],
            }),
            ..Default::default()
        };
        let compiled = CompiledFilter::compile(&f).unwrap();
        assert_eq!(compute_matches(&entries, &compiled), vec![0]);

        let negated = FilterSpec {
            group: Some(RuleGroup {
                logic: "AND".to_string(),
                conditions: vec![Condition {
                    negate: true,
                    ..condition("tag", "regex", "^(Choreographer|OpenGLRenderer)$")
                }],
            }),
            ..Default::default()
        };
        let compiled = CompiledFilter::compile(&negated).unwrap();
        assert_eq!(compute_matches(&entries, &compiled), vec![1]);
    }

    #[test]
    fn invalid_regex_is_reported() {
        let f = FilterSpec {
            search: "(".to_string(),
            is_regex: true,
            ..Default::default()
        };
        assert!(CompiledFilter::compile(&f).is_err());
    }
}
