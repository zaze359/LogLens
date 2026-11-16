# Database Guidelines

> This project has **no database**. This file documents the actual persistence and in-memory
> data model so nobody invents a DB layer.

---

## Overview

There is no SQL, ORM, migration framework or transaction API. Data lives in two places:

| Store | Owner | Lifetime |
|-------|-------|----------|
| In-memory sessions | `AppState` (`session.rs`) | Process lifetime; up to `MAX_OPEN_SESSIONS = 4` log sessions |
| `rules.json` | `config_store.rs` | Persisted in the OS app-config dir, e.g. `~/Library/Application Support/com.zaze.loglens/rules.json` |
| Built-in templates | `templates.json` embedded by `builtin.rs` | Compile-time constant, never written to disk |

The log files being analysed are **read-only inputs**. The app never writes back to them.

---

## In-Memory Model

```rust
pub struct AppState {
    pub sessions: Mutex<HashMap<String, LogSession>>,
    pub streams: Mutex<HashMap<String, DeviceStream>>,
    pub network_captures: Mutex<HashMap<String, AnyCapture>>,
    pub network_sessions: Mutex<HashMap<String, NetworkSession>>,
    pub proxy_guards: Mutex<HashMap<String, ProxyGuard>>,
    pub import_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    seq: AtomicU64,
}
```

Rules:

- Session ids come from one monotonic counter (`s1`, `s2`, ...). Ordering is derived from the
  id; never from `HashMap` iteration order.
- `LogSession` keeps `entries: Vec<LogEntry>` plus a `HashMap<String, Vec<u32>>` match cache.
  The cache key is the serialized filter with `order` cleared (and `search` cleared in `find`
  mode), because those fields do not change the matched set.
- Any append invalidates the whole match cache (`append_all`), since every cached index list
  was computed against a shorter vector.
- Cache size is bounded (`MAX_CACHED_QUERIES = 12`); overflow clears rather than evicts.
- Session eviction at `MAX_OPEN_SESSIONS` must also remove and stop the session's live stream.

---

## Persistence: `rules.json`

Format (v2):

```json
{ "version": 2, "categories": [...], "rules": [...] }
```

- `RULES_FILE_VERSION = 2` in `config_store.rs`.
- Only user-defined categories and custom rules are written. Built-in categories/templates
  are merged in `view()` and are never persisted.
- `normalise()` runs on **both** load and save. It:
  - drops builtin rules and builtin category ids
  - drops blank ids and duplicate category ids
  - re-points rules whose `categoryId` no longer exists to the uncategorised bucket (`""`)
- Writes are whole-file (`fs::write` after `serde_json::to_string_pretty`). This is a
  single-user desktop file; last writer wins. There is no temp-file + rename step today. If a
  future format becomes hard to reconstruct, add atomic write (temp + rename) before adding
  in-place mutation.
- The file path is exposed to the UI (`rules_location`) so users can back it up manually.

---

## Schema Evolution (the "migrations" story)

There is no migration runner. Tolerance is built into the parser:

1. `parse_document` accepts a v2 object, a v1 object (`{version:1, rules}`), or a bare
   `Vec<Rule>` array produced by older exports.
2. Fields are read individually; a malformed/missing optional field falls back to default
   instead of aborting the whole file.
3. Every wire struct uses `#[serde(default)]`, so removed fields are ignored. A regression
   test (`parses_v2_document`) keeps a stale `"package"` key to prove old files still load.
4. `version` is written but not used to branch parsing; the parser is shape-driven.

When changing the format:

- Keep old shapes parseable, or bump the version and write an explicit branch.
- Add a test with a literal old document (see `config_store.rs` tests).
- Run `normalise()` on the result so stale ids cannot reach the UI.

---

## Naming Conventions

- JSON keys are camelCase on the wire (`#[serde(rename_all = "camelCase")]`).
- Rust fields are snake_case (`category_id`, `case_sensitive`).
- Ids are opaque strings. Built-in **rules** use the `builtin-` prefix; built-in
  **categories** use `cat-` and are protected by `builtin_category_ids()`. `normalise()`
  filters on both the `builtin` flag and these id conventions.
- Empty string means "unset": `Rule.category_id == ""` is the uncategorised bucket
  (`UNCATEGORIZED` on the frontend).

---

## Common Mistakes

1. **Persisting built-in rules/categories** — they would come back as user data and could not
   be updated by a new build. `normalise()` exists to prevent this.
2. **Assuming `HashMap` order** — session eviction and any "first/last" logic must derive
   order from the monotonic id.
3. **Forgetting to invalidate the match cache** after appending entries.
4. **Holding a `MutexGuard` across an `await` or a long blocking call** — the UI thread and
   reader threads will stall.
5. **Writing the rules file from a background panic path** — persistence is only done by
   `config_store`, and only from a command.
6. **Adding a database/ORM dependency for one small config file** — JSON is the whole
   requirement.
