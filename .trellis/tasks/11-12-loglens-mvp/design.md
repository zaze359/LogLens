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
| Device logs come from the **platform `adb` binary** (`std::process`), not a USB/native library | Users already have adb, drivers and pairing set up; linking libusb + a device protocol would add a native dependency and still need adb for wireless. Discovery order is `ADB_PATH` → `ANDROID_HOME` / `ANDROID_SDK_ROOT` → `PATH` → common install paths, so a Platform-Tools install is found even when the shell env was never configured | Requires adb on the machine (surfaced as actionable guidance, never a silent failure). Every short call is bounded by a 10s timeout; on Windows the child is spawned with `CREATE_NO_WINDOW` so no console flashes |
| **Only** live capture — no one-shot dump command, no `mode` / `tail_lines` options | `adb logcat` replays the existing device buffer on start, so a live stream already covers "the problem just happened". The dump's only distinct behaviour was freezing silently, which the user read as "the client stopped updating" (two separate reports: "连接上后 UI 上的日志信息并不会实时变化", "没找到 实时抓取"). One mode removes a whole class of confusion | Nothing reads the buffer "as of now" without leaving a child process behind — stopping the stream is the equivalent, and the collected entries stay |
| Live capture = long-lived child + reader thread + `Arc<Mutex<StreamBuffer>>`; IPC polls and drains the buffer | The reader thread must never hold the session mutex while blocked on a pipe, and the session lives in Tauri managed state. Draining on a timer also batches IPC into ~800ms ticks instead of one call per line | New lines reach the UI with up to one poll interval of latency |
| The reader publishes a batch on **16 lines OR 120ms**, whichever trips first | A count-only threshold is wrong for a log tail: it is latency-bound, not throughput-bound. An idle phone emits ~14 lines/s, so "every 128 lines" froze rows *and* the counters riding along with the flush for ~9 seconds — reported by the user as "连接上后 UI 上的日志信息并不会实时变化". The time check runs only when a line arrives, i.e. exactly when there is something to publish | One mutex acquisition per arriving line or per 120ms; uncontended, so unmeasurable. A silent device still yields no update because there is nothing to show |
| `IncrementalParser` always **holds back the trailing entry** | Continuation lines (stack frames, `Caused by:`, long JSON) arrive after the line that opened the entry. Emitting eagerly would split a stack trace from its error line — the one thing this tool exists to prevent | The last line lags one entry behind until the next line arrives or the stream ends; `finish()` flushes it |
| `poll_device_stream` takes the two locks **sequentially, never nested**, and recomputes facets only when entries were appended | Removing the nested-lock window eliminates the only possible lock cycle between reader/drain/session paths. Facets are what the filter dropdowns show, so they must grow with the stream — but idle polls stay cheap | Two lock acquisitions per poll instead of one; facet recomputation is O(entries) on growing polls only |
| Frontend appends only the delta (`offset = entries.length`) while streaming, and re-queries page 0 in descending order | Live lines always land at the end in ascending order, so the loaded page stays valid. In descending order they land on top, where an append would be wrong | A filter change during a stream still resets to page 0, which is the expected behaviour |
| `FileInfo` gains `source` / `device` / `streaming` instead of a second contract | File and live device sessions both normalize to one `FileInfo`, so every existing command (query, export, facets) works unchanged and the UI just renders a badge | Three extra wire fields; `source` is a string, so an unknown value must degrade gracefully (it does: no badge) |
| "先清空设备缓冲区" defaults to **off** | `adb logcat -c` wipes the device buffer, and those lines are usually exactly what the user opened the dialog to look at. Destroying evidence by default is not a sane trade for cleaner logs | Users who want a clean run must tick the box once per session; the checkbox keeps the last choice so a repeat debugging session is one click |
| Package is **reconstructed from the pid**, not read from the line: `ps -A -o PID,NAME` snapshot (start + every 3s) merged with `am_proc_start` / `am_kill` / `Start proc` / `Killing` rows learnt from the stream | `-v threadtime` has no package column, but the pid is always there and an app process is *named* after its package. A `ps` snapshot alone misses processes that already died — which is exactly the crash case — and the events/main rows survive in the ring buffer after death, so the two sources cover each other. Doing it in the parser (not in the adb layer) also gives imported files a package dimension for free | Rows whose process started before the buffer and never appears in it stay unnamed; `system_server` / `logd` / `vendor.*@*` have no package and are deliberately left empty (`is_app_package`) so the package filter stays about applications; `ps` costs ~120ms per snapshot |
| `LogEntry.package` priority: in-line package column > accessibility foreground event > pid mapping | `1234-5678/com.foo` is authoritative. An `onTopAppChanged: com.zui.launcher/...` row is printed by systemui but *describes* the launcher, and the built-in Launcher rules match on `package` — letting the pid mapping win would silently break them | One field carries two closely related meanings; `component` is what distinguishes the foreground-event case |
| Quick filter gains `packages` (multi-select, OR) alongside `levels` / `tags` / `pids` | The package facet existed server-side but was never reachable from the UI, so "package dimension" was true of the data and false of the product. One app, several apps, or app-vs-noise is the first cut anyone makes on an Android log | A hand-written rule can already filter on `package`; the quick filter is for the 2-second case |
| Multi-session is a **frontend-only change**: one tab = one backend session (`sessionId` is the tab id) | Every IPC command was already keyed by `sessionId` and `AppState` already held a session map plus a stream map, so the backend needed no new command. The old behaviour ("opening a file kills the stream") came entirely from `teardownCurrentSession()` in the store | Sessions are capped at 4 (`MAX_OPEN_SESSIONS`); the tab cap is aligned to it so Rust never has to evict behind the UI's back |
| The store keeps **flat mirror fields** for the active tab, and tabs snapshot on switch-away | Every component reads `store.filter` / `store.entries` / `store.selected`. Re-plumbing them all to a tab-scoped accessor would touch every file for no user-visible gain. Snapshotting on the way out plus restoring on the way in keeps one source of truth per moment | Duplicated representation: correctness depends on always snapshotting before switching. `activateTab` is the only switch path and always snapshots first; `filter` is deep-cloned so two tabs never share one object |
| Background tabs **drain their stream but do not render it**: the 800ms tick polls every tab, only the visible one appends to the table | Polling is what moves entries out of the reader's buffer into the session. Skipping hidden tabs would let an unattended capture grow that buffer without bound, and the user would come back to a table frozen mid-burst. Not rendering keeps cost O(1) per hidden tab | A returning tab needs `catchUp()` (repeated delta queries) — capped at 12 pages so one switch cannot fan out into dozens of queries |
| Every async response is validated against `requestSeq`, which is bumped on each tab switch | A poll/query issued for tab A can resolve after the user is already looking at tab B; without the guard its rows would land in B's table. Switching tabs invalidates in-flight work wholesale instead of tagging every callback | Bumping the counter also means a stale `loading` flag must be reset explicitly on switch, otherwise the new tab hangs on "解析中…" |
| `closeTab` removes the tab from the strip **first**, then releases the backend session | Closing should feel instant; `close_session` also kills that tab's `adb logcat`, which can take a moment. The reverse order left the strip frozen on a click that had already happened | A failure to release the session only leaks a slot until the app exits; `MAX_OPEN_SESSIONS` eviction covers it |
| Session eviction picks the oldest by **parsing the numeric id**, not `HashMap::keys().next()` | Ids come from a monotonic counter (`s0`, `s1`, ...), while `HashMap` iteration order is arbitrary — the old code claimed to evict "the oldest" and evicted a random session, which under the old one-session UI was invisible and under tabs would silently kill the wrong capture | Requires ids to stay `s<number>`; parsing failure degrades to "evict something" instead of panicking |
| **One live capture per device**, enforced in `start_device_stream` *and* the store | Two `adb logcat` children replay and follow the same ring buffer, doubling both device load and parsing; worse, `clear_before` on the second capture wipes the lines the first one is still collecting. The user stated this as a hard rule after multi-tab made it reachable | Switching buffers on a running device now requires stopping first. The check is duplicated on purpose: the store check points at the owning tab (good message), the backend check is authoritative (covers any future caller) |
| The device check runs **before** `clear_before`, and is re-run under the lock that publishes the stream | Clearing first and rejecting after would already have destroyed the other capture's data. The first check happens before `spawn` (which takes ~150ms, long enough for a concurrent call to slip through), so the insert path re-checks | Two lock acquisitions on a path that runs once per capture — irrelevant |

| Keep `bundle.icon: []` for dev, and auto-generate `icons/icon.png` via `scripts/gen-icon.cjs` from `postinstall` / `dev` / `build` npm scripts and both setup scripts | `generate_context!()` reads that PNG at compile time even with an empty icon list, so a fresh clone compiles with no checked-in binary; generating it explicitly (not only via Tauri's `beforeDevCommand`) avoids a race with `cargo` startup and makes `--no-run` leave a compilable tree | Official packaging still needs `npm run tauri icon <png>` plus a filled icon array; setup scripts do that automatically for `--build`. A bare `cargo build` on a tree where npm install never ran still needs `node scripts/gen-icon.cjs` once |

## Format compatibility

Seven regex formats are tried in order:

1. year-prefixed plain threadtime — `2026-11-12 17:28:02.013  1080  1161 D Tag: msg`
2. plain threadtime — `11-12 17:28:02.013  1080  1161 D Tag: msg`
3. year-prefixed `pid-tid/package` threadtime (also `-v time`)
4. `pid-tid/package` threadtime
5. `-v long` header, 6. `-v process`, 7. brief/tag (`L/Tag: msg`)

**Why 1–2 come first**: `adb logcat -v threadtime` on a real device prints
`PID TID L Tag: message` — no package column, and a space instead of the slash. The
original parser only knew the `pid-tid/package D/Tag:` shape, so a real phone produced
334 unparsed lines out of 336 (caught by the ignored real-device test, not by the sample
file). Ordering the plain forms first also fixes a subtler bug: with a lazy `middle`, a
message containing something like `E/http: 500` would be re-parsed as an entry whose tag
is `http`, pushing the real tag into `middle`.

pid/tid/package are extracted from the middle segment with a separate pass so uid prefixes
and `pid-tid/package` variants do not break the match; patterns that spell out `pid`/`tid`
override that heuristic. `--------- beginning of ...` becomes a synthetic `S/logcat` entry.

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
- Device capture was added the other way round: instead of a `LogSource` trait, each entry
  point registers into the same `LogSession` and returns the same `FileInfo`, so the
  frontend contract stayed stable without introducing an abstraction with a single
  implementation. A trait is worth adding once a third source (directory watcher, network)
  actually needs shared lifecycle handling.
- Rollback for the device feature alone: drop `adb.rs` / `stream.rs`, the six adb commands
  and `DevicePanel.vue`; file-based usage is untouched.
