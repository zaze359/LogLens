//! Built-in quick templates and their categories.
//!
//! Templates live in `src-tauri/templates.json` and are embedded into the binary at
//! compile time with `include_str!`, so no runtime data file is needed. They use exactly
//! the same `Rule` shape as user-defined rules, which means a template can be applied,
//! edited and saved as a custom rule from the UI.
//!
//! Why a separate JSON file: embedding the JSON in a Rust raw string `r#"..."#` breaks
//! on colour values such as `"#f87171"` — the `"#` sequence is the raw-string closing
//! delimiter, so the literal terminates early and the remaining JSON is parsed as Rust
//! code ("prefix ... is unknown" errors). An external file has no such escaping rules.
//!
//! Regexes deliberately avoid backslashes (`[0-9]` instead of `\d`, `[.]` instead of
//! `\.`) to keep the JSON readable.

use crate::model::{Rule, RuleCategory};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
struct TemplateFile {
    #[serde(default)]
    categories: Vec<RuleCategory>,
    #[serde(default)]
    templates: Vec<Rule>,
}

const TEMPLATES_JSON: &str = include_str!("../templates.json");

fn file() -> TemplateFile {
    serde_json::from_str::<TemplateFile>(TEMPLATES_JSON).unwrap_or_default()
}

pub fn templates() -> Vec<Rule> {
    file().templates
}

/// Built-in categories are compiled in and cannot be edited or removed. They are pure
/// classifications by topic — a category never binds to an application package.
/// User-defined categories live in the rules file and are merged in `commands.rs`.
pub fn categories() -> Vec<RuleCategory> {
    file()
        .categories
        .into_iter()
        .map(|mut c| {
            c.builtin = true;
            c
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logcat::LogcatParser;
    use crate::model::{FilterSpec, LogEntry};
    use crate::rules::{compute_matches, CompiledFilter};

    fn hits_for(entries: &[LogEntry], template_id: &str) -> Vec<u32> {
        let template = templates()
            .into_iter()
            .find(|r| r.id == template_id)
            .unwrap_or_else(|| panic!("missing built-in template {template_id}"));
        let filter = FilterSpec {
            group: Some(template.group),
            ..Default::default()
        };
        let compiled = CompiledFilter::compile(&filter).unwrap();
        compute_matches(entries, &compiled)
    }

    #[test]
    fn templates_are_valid_and_have_unique_ids() {
        let items = templates();
        assert!(!TEMPLATES_JSON.trim().is_empty());
        assert!(items.len() >= 8, "built-in templates failed to parse");
        let mut ids = std::collections::BTreeSet::new();
        for rule in items {
            assert!(rule.builtin);
            assert!(!rule.group.conditions.is_empty());
            assert!(ids.insert(rule.id), "duplicate template id");
        }
    }

    #[test]
    fn template_regexes_compile() {
        for rule in templates() {
            for c in rule.group.conditions {
                if c.op == "regex" {
                    assert!(
                        regex::Regex::new(&c.value).is_ok(),
                        "invalid regex in {}: {}",
                        rule.id,
                        c.value
                    );
                }
            }
        }
    }

    #[test]
    fn categories_are_valid_and_every_template_is_categorised() {
        let categories = categories();
        assert!(categories.len() >= 3, "built-in categories failed to parse");
        let mut ids = std::collections::BTreeSet::new();
        for c in &categories {
            assert!(c.builtin);
            assert!(!c.id.trim().is_empty());
            assert!(!c.name.trim().is_empty());
            assert!(ids.insert(c.id.clone()), "duplicate category id");
        }
        for rule in templates() {
            assert!(
                ids.contains(&rule.category_id),
                "template {} points at missing category {}",
                rule.id,
                rule.category_id
            );
        }
    }

    #[test]
    fn launcher_template_matches_a_real_top_app_event() {
        let line = "11-03 18:02:29.792 I/AccessibilityData( 2108): onTopAppChanged: com.zui.launcher/com.zui.quickstep.RecentsActivity";
        let parsed = LogcatParser::new().unwrap().parse_lines([line]);
        assert_eq!(parsed.entries.len(), 1);

        categories()
            .into_iter()
            .find(|c| c.id == "cat-launcher")
            .expect("Launcher category must exist");

        let template = templates()
            .into_iter()
            .find(|r| r.id == "builtin-launcher-top-app")
            .expect("Launcher top-app template must exist");
        assert_eq!(template.category_id, "cat-launcher");
        assert_eq!(hits_for(&parsed.entries, "builtin-launcher-top-app"), vec![0]);
    }

    #[test]
    fn launcher_template_matches_events_from_any_observed_app() {
        // `<packageName>/<className>` covers arbitrary apps, and className may be a
        // non-Activity component such as android.widget.FrameLayout.
        let parsed = LogcatParser::new().unwrap().parse_lines([
            "11-03 18:02:29.830 I/AccessibilityData( 2108): onTopAppChanged: com.hljzjx.edu.student.launch/com.hljzjx.launch.ui.main.MainActivity",
            "11-03 18:25:05.005 I/AccessibilityData( 2108): onTopAppChanged: com.hailiang.xxb.web.app/android.widget.FrameLayout",
        ]);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(hits_for(&parsed.entries, "builtin-launcher-top-app"), vec![0, 1]);
        assert_eq!(hits_for(&parsed.entries, "builtin-launcher-widget"), vec![1]);
    }

    #[test]
    fn launcher_template_matches_sample_file() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../samples/logcat-sample.txt");
        let parsed = LogcatParser::parse_file(&path).unwrap();
        let hits = hits_for(&parsed.entries, "builtin-launcher-top-app");
        assert_eq!(hits.len(), 5, "sample file must contain 5 top-app events: {hits:?}");
        let widget_hits = hits_for(&parsed.entries, "builtin-launcher-widget");
        assert_eq!(widget_hits.len(), 1, "only the FrameLayout event is a widget");
        assert!(
            parsed.entries[widget_hits[0] as usize]
                .component
                .contains("FrameLayout")
        );
    }
}
