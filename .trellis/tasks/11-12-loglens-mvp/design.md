# Design: LogLens MVP

## Boundaries

```
Vue 3 (webview)                  Rust (Tauri backend)
----------------                 ----------------------
UI state / filters  --invoke-->  commands.rs
virtual table      <-- JSON --   session.rs (in-memory entries + match cache)
                                 logcat.rs (streaming parser)
                                 rules.rs (compiled filter)
                                 builtin.rs / config_store.rs (rules & templates)
```

- **Frontend** never touches the filesystem. It only calls IPC commands and renders.
- **Rust** parses, filters, paginates, exports and persists. This keeps large files out of
  the JS heap and avoids granting broad filesystem permissions to the webview.
- **Single source of truth for the wire format**: `src-tauri/src/model.rs` with
  `#[serde(rename_all = "camelCase", default)]`; `src/types.ts` mirrors it.

## Data flow

1. `open({ ... })` (dialog plugin) or `onDragDropEvent` yields an absolute path.
2. `open_log_file(path)` streams the file with `read_until(b'\n')`, parses each line,
   merges continuations, collects facets (levels / tags / pids / packages) and stores a
   `LogSession` under a generated session id. At most 4 sessions are retained.
3. `query_logs(sessionId, filter, offset, limit)` compiles the filter (regexes included),
   computes/caches matched indices, derives level + tag statistics and returns one page.
4. Frontend renders `entries` with a fixed-row-height virtual list and calls
   `loadMore()` when the scroll position approaches the bottom.

## Key decisions

| Decision | Rationale | Tradeoff |
| --- | --- | --- |
| Parse once into `Vec<LogEntry>` in Rust | Filtering is a fast in-memory scan; no repeated IO | Memory proportional to file size |
| Merge indented lines into the previous entry | A crash hit keeps its stack trace; no context loss | Message can contain `\n`; UI shows first line + `+N` |
| Preserve unparsed lines as `?` entries | Analysis tools must never silently drop data | Slight noise in results |
| Structured conditions instead of a free-form query language | Editable in a form, serialisable, testable | Less expressive than a full DSL |
| Cache matched indices keyed by the serialised filter (excluding `order`) | Paging and sorting are instant after the first scan | Cache invalidation is coarse (clear after 12 entries) |
| Built-in templates as `src-tauri/templates.json`, embedded via `include_str!` in `builtin.rs` | Adding a template needs no Rust logic; still compiled into the binary | JSON escaping must be checked by unit tests. Do **not** inline the JSON in a Rust `r#"..."#` literal: colour values such as `"#f87171"` contain the `"#` closing delimiter and terminate the string early |
| Rules in the OS app config dir via Rust std::fs | No fs plugin / capability scope needed | Not synced across machines (JSON import/export covers this) |
| `RuleCategory` is a first-class entity; `Rule.categoryId` points at it | Built-in categories group shipped templates by topic; user categories are free-form classifications (per app, per module, per problem type). Naming it `categoryId` avoids colliding with `RuleGroup` (a set of conditions) | One category per rule (no multi-tagging); deletion must reparent rules to `""` (uncategorised) rather than drop them |
| Categories are **pure classification** — no `package` field, no auto-matching against the opened log | The user explicitly asked not to bind categories to a package: the same app can be split across categories, and a category can span apps. Keeping the field would leave dead UI and imply semantics that do not exist | No "current app" auto-promotion; the app dimension lives in the `package` log field instead, which rules can filter on |
| Structure accessibility foreground events in `build_entry`: `<packageName>` → `LogEntry.package`, `<className>` → new `LogEntry.component` | The data comes from `AccessibilityEvent.packageName` / `className`; the part after `/` is **not** necessarily an Activity (it can be `android.widget.FrameLayout`, a dialog or popup). Making both sides first-class fields lets rules filter per app and per component class without touching the parser again. `collect_facets` needs no special case because `package` is already populated | Adds an optional field to the generic wire model; recognised event keys are a heuristic (`onTopAppChanged`, `topResumedActivity`, `mCurrentFocus`, `ResumedActivity`, `topActivity`) |
| Built-in `Launcher` category (a plain topic bucket) with two templates: all `onTopAppChanged` events, and widget/dialog-only ones filtered on `component` | Groups the desktop / foreground-app rules without binding any package. The main template does **not** pin tag, package or component: observed apps vary (`com.zui.launcher`, `com.hljzjx.edu.student.launch`, `com.hailiang.xxb.web.app`, ...) | Built-in category definitions are read-only (custom rules can still be added into them). The widget template matches class names heuristically |
| `rules.json` v2 document `{ version, categories, rules }`, parsed tolerantly as object or bare array | Upgrades preserve existing v1 files and old exported arrays; `normalise()` drops built-ins, de-duplicates ids and clears dangling `categoryId`s | Import may regenerate colliding category ids; an id map rewrites imported rules' `categoryId` so the link survives |
| Every mutating rule/category IPC returns `RulesView { categories, rules }` | One round trip refreshes the whole sidebar; built-in categories are merged in `config_store::view()` so the frontend never maintains two lists | Slightly larger payload; built-in templates are still fetched separately |
| Sidebar search and grouping are derived in Vue `computed` | Rule counts are small (tens), so client-side multi-keyword AND search over name/description/category/conditions is instant and needs no IPC | Not suited to thousands of rules; a server-side index would be needed then |
| Keep `bundle.icon: []` for dev, and auto-generate `icons/icon.png` via `scripts/gen-icon.cjs` from `postinstall` / `dev` / `build` npm scripts and both setup scripts | `generate_context!()` reads that PNG at compile time even with an empty icon list, so a fresh clone compiles with no checked-in binary; generating it explicitly (not only via Tauri's `beforeDevCommand`) avoids a race with `cargo` startup and makes `--no-run` leave a compilable tree | Official packaging still needs `npm run tauri icon <png>` plus a filled icon array; setup scripts do that automatically for `--build`. A bare `cargo build` on a tree where npm install never ran still needs `node scripts/gen-icon.cjs` once |

## Format compatibility

Five regex formats are tried in order: year-prefixed threadtime, classic threadtime
(also `-v time`), `-v long` header, `-v process`, then brief/tag. pid/tid/package are
extracted from the middle segment with a separate pass so uid prefixes and
`pid-tid/package` variants do not break the match. `--------- beginning of ...` becomes
a synthetic `S/logcat` entry.

## Error handling

- All commands return `Result<T, String>`; errors surface as a banner in the UI.
- Invalid regexes are rejected during filter compilation, before any scanning.
- Invalid UTF-8 is replaced lossily so one bad byte cannot abort the whole file.
- IPC responses are guarded by a monotonically increasing request sequence so a slow
  older query cannot overwrite a newer result.

## Rollout / rollback

- Feature is additive: no existing application code was replaced (empty repository).
- Rollback = remove `src/`, `src-tauri/`, `samples/`, root config files; `.trellis/`
  scaffolding is untouched.
- Future log sources should be introduced behind a `LogSource` abstraction in the Rust
  layer so the frontend contract (`open_log_file` → `FileInfo`) stays stable.
