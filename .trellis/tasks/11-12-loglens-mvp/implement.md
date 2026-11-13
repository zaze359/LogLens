# Implement: LogLens MVP

## Checklist

- [x] Scaffold Tauri 2 + Vue 3 + Vite + TypeScript (`package.json`, `vite.config.ts`,
      `tsconfig.json`, `tauri.conf.json`, `capabilities/default.json`, `Cargo.toml`).
- [x] Define shared wire models (`model.rs` / `types.ts`) with camelCase serde.
- [x] Implement Logcat parser with multi-format regexes, continuation merging, divider
      handling, lossy UTF-8, facets and unit tests.
- [x] Implement rule engine (compiled conditions, AND/OR, negation, regex validation),
      quick filters, time normalisation and unit tests.
- [x] Add 9 built-in templates as JSON with validity tests.
- [x] Implement session store, paged query, cache, statistics and export.
- [x] Implement custom-rule persistence, import/export, and all IPC commands.
- [x] Implement Vue UI: top bar, filter bar, rule sidebar, condition editor, virtual log
      table, detail drawer, stats panel, toast/banner feedback.
- [x] Add sample log file, README, PRD/design/research artefacts.
- [x] Add one-shot setup scripts (`scripts/setup.sh` for macOS/Linux, `scripts/setup.ps1`
      for Windows): environment detection, automatic installation of missing toolchains,
      dependency install and dev/build/test launch; npm shortcuts in `package.json`.
- [x] Fix compile-time default window icon: `scripts/gen-icon.cjs` generates
      `src-tauri/icons/icon.png` (zero-dependency, idempotent 64x64 PNG) from
      `postinstall` / `dev` / `build` npm scripts and both setup scripts, so
      `generate_context!()` works on a fresh clone with `bundle.icon: []`.
- [x] Eliminate release-build warnings: drop the redundant `LogSession::info` field and
      make `LogcatParser::parse_lines` test-only.
- [x] Rule categories + search: `RuleCategory` (pure classification, no package binding)
      and `Rule.categoryId` in `model.rs` / `types.ts`; 4 built-in categories in
      `templates.json`; `RulesView` IPC; `rules.json` v2 with v1 / bare-array migration,
      `normalise()` and `view()` with unit tests.
- [x] Category CRUD commands (`list_rule_bundle`, `save_rule_category`,
      `delete_rule_category`), category-aware import/export with id remapping.
- [x] Vue sidebar rewrite: search box, collapsible category sections, category editor
      dialog, grouping selector in the rule editor.
- [x] First app-specific preset: built-in `Launcher` category (a plain topic bucket — the
      initial `package: com.zui.launcher` binding was removed after the user asked for
      categories as pure classification) plus
      "前台应用 / 页面变更" and "弹窗 / 非 Activity 组件" templates. Accessibility events
      are structured as `package` = `AccessibilityEvent.packageName` and new `component`
      = `AccessibilityEvent.className` (exposed in `FIELDS`, rule engine and detail drawer);
      5 matching lines in `samples/logcat-sample.txt`; end-to-end tests parse three real
      log lines and the sample file and assert template hit counts.

- [x] Device log source: `adb.rs` (discovery via `ADB_PATH` / `ANDROID_HOME` /
      `ANDROID_SDK_ROOT` / `PATH` / common install paths, `adb version` status,
      `adb devices -l` parsing with `device` / `unauthorized` / `offline` /
      `no permissions`, timeout-bounded short calls, Windows `CREATE_NO_WINDOW`,
      logcat/clear argument builders).
- [x] One-shot capture (`capture_device_log`): `adb logcat -d -v threadtime [-b ...] [-t N]`
      straight into a temp file, cleared first when asked, parsed with the file parser.
- [x] Live capture (`stream.rs`): long-lived `adb logcat` child, reader + stderr threads,
      `Arc<Mutex<StreamBuffer>>`, `startup_error()` fast-fail for unauthorised devices,
      `stop()`, `stop_all_streams()` on window destroy.
- [x] `IncrementalParser` in `logcat.rs` holds back the trailing entry so stack traces stay
      attached during streaming; `LogSession::append_all` assigns indices and invalidates
      the match cache.
- [x] IPC: `adb_status`, `list_adb_devices`, `start_device_stream`, `poll_device_stream`,
      `stop_device_stream`; `FileInfo.source/device/streaming`;
      `close_session` and session eviction also kill the child.
- [x] **Parser bug found by the real-device test**: a phone prints
      `11-12 17:28:02.013  1080  1161 D PowerGuruService: msg` for `-v threadtime`
      (no package, space instead of slash). The parser only knew `pid-tid/package D/Tag:`
      and reported 334/336 lines unparsed. Added the two plain-threadtime patterns *first*
      and let explicit `pid`/`tid` groups override the `middle` heuristic; two regression
      tests cover the format and the `E/http: 500` inside-a-message trap.
- [x] Frontend: `DevicePanel.vue` (adb status + install guidance, device picker with state
      labels, buffer chips, optional clear-first), top-bar "连接设备" + source badge + live
      dot + "停止实时抓取", status bar live hint, 800ms polling that appends only the delta
      and refreshes facets, descending order re-queries page 0.
- [x] **Live UI fix (reported by the user)**: the reader flushed only every 128 lines, so an
      idle phone (~14 lines/s) froze the table and the counters for ~9s at a time. Flush is
      now "16 lines OR 120ms", poll is 800ms, `LogTable` auto-follows the tail while
      streaming (and shows "↓ 回到最新" once the user scrolls up), and `paging` guards the
      next-page offset against the poll tick racing a scroll-triggered `loadMore`.
      Verified by `steady_stream_delivers_a_probe_line_within_a_second` (writes a line via
      `adb shell log`, asserts delivery < 1.5s) and `poll_append_and_query_round_trip`.
- [x] **Discoverability fix (reported as "没找到 实时抓取")**: live capture was a radio pair
      below the fold and the default was the one-shot dump, so the first capture froze by
      design and looked broken. Moved the choice above the fold, made live the default, and
      added a top-bar shortcut to the live mode. *(Superseded by the next item.)*
- [x] **Single mode only (user: "近保留实时抓取即可，且默认不勾选清空缓冲区")**: two modes
      turned out to be one mode plus a foot-gun. `adb logcat` already replays the existing
      device buffer on start, so a live stream covers "look at what already happened" too,
      and the dump's only unique behaviour was freezing silently. Removed
      `capture_device_log`, `dump_to_file`, `DeviceCaptureOptions.mode/tail_lines`, the
      segmented control, the line cap, `store.deviceMode`, `captureFromDevice` and the
      top-bar "转为实时抓取". `clearBefore` now defaults to `false` — `adb logcat -c`
      destroys the evidence the user just opened the dialog to look at. The dialog carries
      a one-line live note explaining the replay instead of a mode switch.
- [x] **Package dimension (user: "locat 里面确实 package 的维度")**: the facet was collected
      but almost always empty — `-v threadtime` has no package column, so only the rare
      accessibility foreground event ever filled it, and `FilterSpec` had no `packages` field
      at all, meaning the UI could not filter by package even when the data was there.
      Added `ProcessIndex` (`logcat.rs`): a `ps -A -o PID,NAME` snapshot taken before the
      child starts and refreshed every 3s from the reader thread (only when lines arrive),
      plus pid → package learned from `am_proc_start` / `am_kill` / `Start proc` /
      `Killing` / `Process ... (pid N) has died` rows. Death rows drop the mapping so a
      recycled pid does not inherit a dead app's name. `com.tencent.mm:push` collapses to
      `com.tencent.mm`; `system_server` / `logd` / `vendor.*@*` are deliberately left
      unnamed (`is_app_package`) so the filter stays about applications. Precedence: in-line
      package > foreground-event package > pid mapping. It lives in the parser, so imported
      files get the same treatment from `Start proc` rows. Exposed end to end:
      `FilterSpec.packages` (multi-select OR) in Rust + TS, a 包名 dropdown with removable
      chips in `FilterBar.vue`, and a 包名 column between TID and Tag in `LogTable.vue`.
      Verified on a real device: 9183 entries → 3060 named, 19 distinct packages, including
      `com.hailiang.workcloud.student`.
- [x] **Multi-tab (user: "支持多窗口…会关闭之前的抓取，不方便同时分析多个日志")**: the
      backend already keyed everything by `sessionId` and already held 4 sessions, so the
      single-session limit was purely `teardownCurrentSession()` in the store. Replaced it
      with `SessionTab[]`: `openTab` / `activateTab` / `closeTab`, snapshot-on-switch-away
      plus restore (the flat `store.xxx` fields stay, so no component needed rewriting),
      `requestSeq++` on every switch to discard in-flight responses from the tab just left,
      deep-cloned filters so two tabs never share one object. The 800ms tick now covers all
      tabs — the visible one appends to its table, hidden ones only drain their backend
      buffer (otherwise an unattended capture grows without bound); returning triggers
      `catchUp()` (≤12 delta pages). Tab strip is a new `SessionTabs.vue` (live dot, entry
      count, per-tab `×`); `LogTable` remembers `scrollTop` per tab. Files dedupe by path,
      tabs cap at `MAX_TABS = 4` = `MAX_OPEN_SESSIONS`, `closeTab` removes the strip entry
      first and releases the session in the background.
- [x] **Eviction bug found while wiring multi-tab**: `register_session` claimed to evict the
      oldest session but used `sessions.keys().next()`, i.e. an arbitrary one — invisible
      with a single visible session, silently fatal once tabs exist. Now derives the oldest
      from the monotonic id (`s0`, `s1`, ...).
- [x] **One live capture per device (user: "同台设备不应能被多个会话实时抓取")**: two
      `adb logcat` children replay and follow the same ring buffer, and `clear_before` on the
      second would wipe what the first is still collecting. `DeviceStream` now records its
      `serial`; `start_device_stream` refuses a second capture before touching the buffer
      (`ensure_device_not_streaming`) and re-checks under the lock that publishes the stream,
      since `spawn` takes long enough for a concurrent call to slip past the first check.
      The store checks first so it can *point at* the owning tab instead of just failing.
- [x] **Stale-stream bug exposed by that rule**: when a background tab's stream ended on its
      own (unplugged device), only the active-tab path called `stop_device_stream`; the
      backend kept the dead entry and the device became permanently un-capturable. Both the
      background success and failure paths now release it.
- [x] **Import progress (user: "增加一个加载进度的效果，而不是什么反应都没有，卡在那")**: the
      freeze was structural — `open_log_file` was a *synchronous* Tauri command, and Tauri runs
      those on the main thread, so a 300k-line parse stopped the webview from painting at all.
      Now an `async` command that does the read on `spawn_blocking`, emitting
      `log-open-progress` with `{ requestId, bytesRead, totalBytes, entries, phase }` throttled
      to 100ms (`parse_file_with_progress` calls back every 4096 lines). Progress is
      **byte-based** because the line count is only known at EOF. `collect_facets` (a second
      full pass) moved onto the same thread and reports `phase: "indexing"`, so the bar does
      not stall at 100%. Cancellation is a per-request `AtomicBool` in
      `AppState.import_cancels`, checked each chunk; the partial result is dropped and no
      session is registered. UI: `ImportOverlay.vue` is a **bottom-right floating card** (a
      modal would just be a prettier freeze), hidden for the first 250ms so small files do not
      flash it, with read/total size, percent, entry count and a cancel button. The store
      guards one import at a time and re-checks `MAX_TABS` after the (slow) import in case a
      device capture took the last slot meanwhile.
- [x] **Full lines without clicking (user: "显示不全，需要点击单行才能看全部…应该可以显示完整，
      左右可以滚动")**: the message column was `minmax(0, 1fr)` + `text-overflow: ellipsis`, so
      anything past the window width was unreadable until you opened the drawer. Rows must stay
      26px tall (fixed height is what makes the virtual list cheap), so the fix is a **wider
      column plus horizontal scrolling**: `--msg-w` is computed from the widest first line among
      the loaded entries (canvas `measureText`; `ch` under-sizes mixed CJK/ASCII) and the grid
      track became `minmax(var(--msg-w), 1fr)`. The header lives outside the scroll container,
      so it is translated by `-scrollLeft` to stay above its columns. Verified the layout in a
      headless browser against an equivalent static page: `scrollWidth` 5253 vs `clientWidth`
      1200, and at max scroll the JSON tail (`"appVersion":"4.2.1"}`) is fully visible.
- [x] **Two bugs caught by measuring instead of assuming**: (a) `since_report` was never reset,
      so after the first 4096 lines the progress callback fired on *every* line — 296k
      callbacks for a 300k-line file; (b) the parse looked like 24k lines/s until I noticed
      `cargo test` builds in the **dev** profile. Release: 29 MB / 300k lines in 1.03s
      (~293k lines/s); dev profile ~12.6s. Both numbers are now in the README, because the
      dev/release gap would otherwise read as "the import is slow".

## Validation commands

```bash
npm install
npm run build                     # vue-tsc --noEmit && vite build
cd src-tauri && cargo test --lib  # parser / rules / templates / adb parsing
cd src-tauri && cargo test --lib -- --ignored   # real device: adb, threadtime, live stream
npm run tauri dev                 # manual acceptance, see README section 11
npm run tauri icon <png>          # required before npm run tauri build
npm run tauri build               # run on macOS and Windows separately
```

### Verified locally on macOS arm64 (2026-11-12)

| Command | Result |
| --- | --- |
| `npm run build` | PASS — `vue-tsc --noEmit` clean; after categories: `index-C1_WnR5e.js` 125.47 kB, `index-DIXdkiNR.css` 11.98 kB |
| `cd src-tauri && cargo build` | PASS — zero warnings; `generate_context!()` no longer fails on the missing icon |
| `cd src-tauri && cargo test --lib` | PASS — 23 passed, 0 failed (incl. 3 real `onTopAppChanged` lines → package/component split, Launcher templates end-to-end, 5 hits + 1 widget hit in the sample file) |
| `bash scripts/setup.sh --check-only` | PASS — all required items OK, only packaging icons WARN |
| `bash scripts/setup.sh --no-run` after deleting `icon.png` | PASS — icon regenerated byte-identically; `postinstall` also ran; `cargo fetch` OK |
| `cd src-tauri && cargo test --lib` (device feature) | PASS — 31 passed, 5 ignored (real-device tests), 0 failed |
| `cd src-tauri && cargo test --lib -- --ignored` (device attached) | PASS — 5 passed on HLKJ_PB_11ZTRS, later re-run 5 passed on HA20ZD1H: adb discovery, real `-v threadtime` parse, live stream drain, probe line within 1s, poll/append/query round trip |
| `cd src-tauri && cargo test --lib` (after pid → package) | PASS — 38 passed, 6 ignored, 0 failed |
| `cd src-tauri && cargo test --lib -- --ignored` (after pid → package, device HA20ZD1H) | PASS — 6 passed; `live_capture_resolves_packages_from_pids` reported 9183 entries, 3060 named, 19 distinct packages (`com.hailiang.workcloud.student`, `com.zui.launcher`, `com.android.systemui`, ...) |
| `cd src-tauri && cargo build` (after device feature) | PASS — zero warnings |
| `npm run build` (after device feature) | PASS — `index-DH6VLY8I.js` 135.88 kB, `index-CiadpH3o.css` 13.72 kB |
| `bash scripts/setup.sh --test` | PASS — all green |
| `cd src-tauri && cargo build` + `npm run build` (after dropping the one-shot mode) | PASS — zero warnings; `index-ZoIBoccA.js` 135.71 kB, `index-VwGs4Gl6.css` 14.39 kB |
| `cd src-tauri && cargo build` + `npm run build` (after pid → package) | PASS — zero warnings; `index-BvYbvpuQ.js` 136.71 kB, `index-pSlZNoV2.css` 14.42 kB |
| `cd src-tauri && cargo test --lib` (after multi-tab + one live capture per device) | PASS — 38 passed, 6 ignored, 0 failed |
| `cd src-tauri && cargo test --lib -- --ignored` (after multi-tab + one live capture per device, device HA20ZD1H) | PASS — 6 passed in 2.44s: adb discovery, real `-v threadtime` parse, live stream drain, probe line within 1s, poll/append/query round trip, pid → package resolution |
| `cd src-tauri && cargo build` + `npm run build` (final) | PASS — zero warnings; `index-Hmk2aMtm.js` 140.74 kB, `index-9k-XKqoh.css` 15.61 kB |
| `bash scripts/setup.sh --test` (final) | PASS — 38 passed / 6 ignored, "全部验证通过" |
| `cd src-tauri && cargo test --lib` (after import progress) | PASS — 40 passed, 6 ignored, 0 failed; new `parse_reports_progress_and_finishes_at_the_file_size` (byte reports monotonic, last report == file size) and `cancelling_stops_the_parse_and_returns_nothing` |
| `cd src-tauri && cargo build` + `npm run build` (after import progress) | PASS — zero warnings; `index-DB7hAwB8.js` 150.71 kB, `index-hDeLLhey.css` 17.22 kB |
| Release parse benchmark (29 MB / 300 420 lines) | PASS — 1.03s, ~293k lines/s, 74 progress callbacks (dev profile: 12.6s) |
| `npm run build` (after full-line display) | PASS — `index-DR3_n2fk.js` 151.44 kB, `index-B5DnUQsy.css` 17.34 kB |
| Headless browser layout check of the table (Edge, 1200px viewport, page mirroring `--msg-w` + header transform) | PASS — `scrollWidth` 5253 / `clientWidth` 1200 (horizontal scroll present), header labels aligned with data columns, line tail reachable at max scroll |

Environment: macOS arm64, Node v24.14.0, npm 11.9.0, rustc/cargo 1.99.1, rustup 1.29.3,
adb 1.0.41 (platform-tools 35.0.2), one USB device attached.
Still pending: GUI acceptance for multi-tab switching / closing, the package column and its
filter, and the "device already capturing" redirect (`npm run tauri dev`); cross-platform
build on Windows (`scripts/setup.ps1`); and `npm run tauri build` with a real icon set.

## Review gates

1. Rust and TypeScript compile without errors; `cargo test --lib` is green.
2. IPC contract: every command used in `src/lib/ipc.ts` is registered in `lib.rs`, and
   every camelCase field in `types.ts` exists in `model.rs`.
3. Manual acceptance passes all scenarios in README section 11 (including the device
   panel, live capture and "no orphan adb process after closing the window").
4. Cross-platform: build on macOS and Windows; dialog plugin requires Rust >= 1.90.

## Rollback points

- Parser change → revert `logcat.rs`; session/rules code depends only on `LogEntry`.
- Rule-engine change → revert `rules.rs`; `session.rs` depends only on `CompiledFilter`.
- UI change → revert the affected `.vue` file; the IPC layer is independent.
- Device source → remove `adb.rs` / `stream.rs`, the six adb commands and the
  `DevicePanel` mount; `open_log_file` and file-driven flows are untouched.
