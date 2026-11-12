# PRD: LogLens MVP — Log Analysis Desktop Client

## Source requirement

Zhao Zhen asked for a client that analyses various collected logs:

1. Desktop client for macOS and Windows, built with **Tauri + Rust + Vue 3**.
2. First log type: **Android Logcat**, with **local files** as the only source for now.
3. **User-configurable rules** for fast filtering, plus **built-in quick templates**.
4. Follow-up: rule templates must support **grouping and search**, because separate
   rule sets will be created for **different applications** later.
5. Follow-up: **extend the log source** beyond local files — connect a real Android
   device and follow its logs over `adb logcat` live, while the operator reproduces the
   problem.
6. Follow-up: keep **only** the live device source (drop the one-shot buffer dump) and
   default "clear the device buffer first" to **off**, because clearing destroys the
   evidence the user wants to read.

## Scope

### In scope

- A single-window desktop application that opens one local text log file (file picker and
  native drag & drop) and parses Android Logcat output.
- **Device source**: detect `adb`, list connected devices with their readiness
  (`device` / `unauthorized` / `offline` / `no permissions`), then follow new lines live
  (`adb logcat`, stop at any time; the existing device buffer is replayed on start, so
  "the problem just happened" is covered too). Choose the buffers (`main` / `system` /
  `crash` / `events`) and optionally clear the device buffer first — **off by default**.
  Device and file sessions share the same parser, rules, filters and export.
- **Multiple log sources at once (tabs)**: opening a file or starting a device capture adds
  a tab instead of replacing the current one. Each tab keeps its own filter, sort order,
  selected row and scroll position; a live capture that is switched away keeps receiving in
  the background and catches up when brought back. Up to 4 tabs, matching the backend
  session cap.
- Structured display of level, timestamp, pid/tid, tag, package and message with stack
  traces preserved.
- Quick filters: level, tag, pid, **package**, keyword / regex search, exclusions, time
  range, sort order.
- **Package dimension**: `adb logcat -v threadtime` carries no package column, so the
  package is reconstructed from the pid (`ps` snapshot plus `am_proc_start` /
  `Start proc` rows) and shown per row, as a filter facet and as a quick filter. App
  sub-processes collapse to their package; processes without a package
  (`system_server`, `logd`, vendor HALs) stay unnamed so the filter is about apps.
- Structured rule editor: field + operator + value, case sensitivity, negation, AND/OR
  combination, live hit-count test.
- At least 8 built-in templates covering the common Android debugging scenarios,
  organised into built-in topic categories.
- **Rule categories** as first-class entities: built-in (read-only) plus user-defined.
  Categories are **pure classification** — they do not bind to an application package,
  so an app can be split across categories and a category can span apps. Create /
  rename / recolor / delete / collapse categories; deleting a category keeps its rules
  (moves them to "uncategorised").
- **Sidebar search** across rule name, description, category name and condition
  fields/values, with multi-keyword AND matching and a hit counter.
- Custom rules + categories persisted locally, importable/exportable as JSON.
- Filtered results exportable as plain text.
- Usable with large single files (virtual scrolling + backend paging + match cache).

### Out of scope (this iteration)

- In-app wireless pairing (`adb pair`), several live captures at once, directory or
  multi-file sessions, compressed archives.
- Non-Logcat formats (iOS, backend access logs, frontend telemetry).
- Full-text indexing, cloud sync, team sharing, authentication.
- Code signing / notarisation / distribution pipeline.

## Acceptance criteria

1. `npm run tauri dev` starts a native window on macOS; `npm run tauri build` produces
   platform bundles on macOS and Windows from the same source.
2. Opening `samples/logcat-sample.txt` parses threadtime (plain / package / uid / year),
   time, brief, long and process formats; unparsed lines are preserved and counted;
   indented stack frames stay attached to their parent entry.
3. Applying each built-in template changes the result set visibly; the crash template
   keeps the full stack trace available in the detail drawer.
4. A custom rule can be created, tested, saved, applied after restart, exported and
   imported without data loss.
5. Exporting filtered results writes exactly the matched entries, in the displayed order.
6. Invalid regexes produce a readable error instead of hiding results or crashing.
7. Unit tests cover parsing, rule evaluation, template/category JSON validity and
   rules-file format migration.
8. A category can be created with just a name / colour / description, rules can be
   assigned to it, and the old v1 `rules.json` / a bare exported rule array still load
   without data loss. Categories never bind to a package.
9. Searching by a category name or a condition value narrows the sidebar to the
   matching rules; clearing the query restores all collapsed state.
10. With a phone attached and authorised, "连接设备" lists it as ready; an unauthorised or
    offline device is shown with that state and cannot be captured.
11. "开始实时抓取" produces a non-empty session tagged `实时` (the device buffer is
    replayed on start), with tag / package facets populated and built-in templates
    matching.
12. A live capture appends new entries within ~2s of them appearing on the device, grows
    the tag / package facets, stops on demand, and stops appending after "停止实时抓取".
13. "先清空设备缓冲区" is unchecked by default; when checked, the session starts from an
    empty buffer.
14. A live capture labels most rows with a real application package (not `—`), and the
    package quick filter narrows to one app, with several selected behaving as a union.
15. Closing the window while streaming leaves no `adb logcat` child process behind.
16. Opening a file while a device capture runs keeps the capture alive: the capture's tab
    shows more entries after switching back than when it was left, and only that tab
    offers "停止实时抓取".
17. Two tabs hold independent filters, sort orders and scroll offsets; switching back and
    forth never mixes them.
18. Closing a tab stops that tab's `adb logcat` child and switches to a neighbour; closing
    the last tab returns to the empty state. A fifth tab is refused with a message that
    names the cap.
19. A device can only be captured once at a time: starting a second live capture on the
    same serial is refused (and the UI switches to the tab that owns it), so a second
    capture's "clear the buffer first" can never wipe a running capture's data.
20. Real-device integration tests (`cargo test --lib -- --ignored`) cover adb discovery,
    `adb devices -l` parsing, a real `-v threadtime` stream parsed by the logcat parser,
    a probe line delivered within 1s, and package reconstruction from pids.

## Constraints

- Rust backend owns all file IO; the webview only receives data over IPC.
- No filesystem plugin scope is granted: arbitrary user-selected paths are read in Rust.
- The JSON contract between Rust and TypeScript is defined by `model.rs` / `types.ts`.
