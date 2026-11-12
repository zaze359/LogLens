# PRD: LogLens MVP — Log Analysis Desktop Client

## Source requirement

Zhao Zhen asked for a client that analyses various collected logs:

1. Desktop client for macOS and Windows, built with **Tauri + Rust + Vue 3**.
2. First log type: **Android Logcat**, with **local files** as the only source for now.
3. **User-configurable rules** for fast filtering, plus **built-in quick templates**.
4. Follow-up: rule templates must support **grouping and search**, because separate
   rule sets will be created for **different applications** later.

## Scope

### In scope

- A single-window desktop application that opens one local text log file (file picker and
  native drag & drop) and parses Android Logcat output.
- Structured display of level, timestamp, pid/tid, tag, package and message with stack
  traces preserved.
- Quick filters: level, tag, pid, keyword / regex search, exclusions, time range, sort order.
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

- adb / live streaming, directory or multi-file sessions, compressed archives.
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

## Constraints

- Rust backend owns all file IO; the webview only receives data over IPC.
- No filesystem plugin scope is granted: arbitrary user-selected paths are read in Rust.
- The JSON contract between Rust and TypeScript is defined by `model.rs` / `types.ts`.
