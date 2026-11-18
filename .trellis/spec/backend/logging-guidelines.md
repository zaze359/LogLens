# Logging Guidelines

> Two separate channels, never mixed: stderr (`eprintln!`) for developers at the terminal,
> and a bounded on-disk **error log** (`error_log.rs`) for failures a bundled-app user can no
> longer see. User-facing problems still travel through `Result` first.

---

## Overview

- No `log`, `tracing` or `env_logger`. `Cargo.toml` intentionally has no logging dependency.
- `eprintln!` goes to the terminal that launched `npm run tauri dev`; in a bundled app it is
  usually invisible. Therefore **stderr is for developers**, not for users.
- Anything the user must see is returned from a command and rendered by the UI.
- Stderr logging is sparse: startup/cleanup, skipped malformed input, and background-thread
  failures. There is no per-entry or per-request logging.

### Client diagnostics log (`error_log.rs`)

- File: `<app_config_dir>/error-log.jsonl` — the same directory as `rules.json`.
- One bounded file carries two kinds of records: **failures** (Rust panics with backtrace,
  uncaught frontend errors via `report_client_error`, Tauri `Err` from `call()`) and
  **`info` flow checkpoints** (see below). There is no second log file to collect.
- Bounded: `MAX_RECORDS = 500`, newest kept; identical records inside 10s fold into one
  with an incrementing `count`. For `info`, the folded record keeps the newest `detail`.
- Deliberately excludes `poll_device_stream` / `poll_network_capture` (`SILENT_COMMANDS` in
  `ipc.ts`): teardown makes them fail routinely, and their real errors already reach the UI
  through `StreamStatus.error` / capture status. Their **successful** polls still produce
  flow checkpoints; only their `Err` is excluded.
- The UI in `ErrorLogPanel.vue` lists / filters / exports / clears it. It is user-visible
  support data, so export is plain text (not JSONL) and the path is shown in the panel.
- Initialising the log or recording into it must never panic or abort startup — a poisoned
  mutex is recovered with `into_inner()`, and init failures fall back to stderr.

### Flow checkpoints (`info` / `source=flow`)

Every user-visible workflow leaves checkpoints in the same file: startup/shutdown, file and
session open/close, device streams, H5/native capture, rules, import/export, proxy restore,
and frontend notices. They exist because "点了没反应 / UI 上看不到" only reports the final
symptom; a checkpoint trail names the last stage reached and the counters at that moment.

- **API**: `error_log::record_flow(scope, message, detail)` in Rust,
  `reportClientFlow(scope, message, detail)` in frontend. Use `flow_echo` when replacing a
  workflow `eprintln!` so the terminal copy survives. Both write `info` / `source=flow`.
- **Scope** is `<module>.<step>`, optionally with an id for per-session steps
  (`app.start`, `file.open`, `device.stream`, `capture.poll.net-3`, `ui.notice`, …).
  `message` names the state; a new module needs no UI change.
- **Coverage**: add a checkpoint at each boundary the user can observe — request accepted,
  ready/completed, state change, cancel, cleanup failure. Aggregate hot loops into one
  boundary summary (`import.parse` records skipped-line count, not one line per row).
- `detail` carries **ids, counts, booleans and short status strings only**. Never URLs,
  query strings, headers, cookies, tokens, bodies, or file paths the user did not choose.
  A status/error already shown in the UI is allowed.
- Identical signatures fold inside 10s with the newest `detail`, so a high-frequency poll
  stays one record. Frontend capture poll additionally de-duplicates by signature and adds
  a 5s heartbeat while running. `reportClientFlow` does not dispatch `loglens-client-error`,
  so checkpoints never trigger the badge or refresh the panel.
- UI filter: level `info` is labelled 流程, source `flow` is labelled 流程
  (`capture` is kept only for records written before this convention). All notices are
  recorded through `setNotice`, so a workflow that already reports to the user is covered
  automatically. Badge counts only non-`info` records. `info` details render inline
  (no click) and scopes map to short stage names, so a successful capture still shows
  "start → proxy → first rows → poll/UI" at a glance.

---

## Log Levels

Diagnostics records use `panic` / `error` / `warning` for failures and `info` for flow
checkpoints. Stderr itself has no levels; use the message to convey severity.

| Situation | Prefix / wording | Example |
|-----------|------------------|---------|
| Startup self-heal | `flow_echo` | `flow_echo("proxy.heal", "启动自愈", &healed)` in `lib.rs` |
| Cleanup failed but the app continues | `flow_echo` + reason | `flow_echo("proxy.restore", "设备代理恢复失败", …)` |
| Skipped malformed external input | boundary summary | `record_flow("import.parse", "跳过损坏的 JSONL 行", "skipped=N")` |
| Test/helper diagnostics | free-form `eprintln!` | `eprintln!("记录到 {} 条请求...", rows.len())` |

Rules:

- Keep one-line, greppable messages. Include the action and the cause.
- Failure of a *user-initiated* action is returned as `Err`, not logged and dropped.
- Test-only `eprintln!` is not a flow checkpoint: it is invisible to shipped users and
  must not write into the diagnostics file.

---

## Structured Logging

There is none. Messages are human-readable strings. If a structured log ever becomes
necessary, prefer adding `serde_json` output to the same `eprintln!` rather than introducing
a logging framework for one call site.

---

## What to Log

- Every user-visible workflow boundary as a flow checkpoint (request → ready/completed,
  state change, cancel, cleanup failure), across startup, files/sessions, device streams,
  capture, rules, import/export and proxy restore.
- Startup self-healing results (`heal_stale_proxies`).
- Device-side cleanup failures that cannot be reported to the UI because the window is going
  away (proxy restore, guard drop).
- Malformed lines in external capture files (JSONL/HAR) — as one aggregate count, not one
  record per line.
- Reader-thread errors after the associated data has been flushed.

## What NOT to Log

The rules below apply to **both** stderr and the client diagnostics log. The diagnostics log
additionally never stores parsed entries, request bodies or file contents — it stores a
message, a scope, counters and optionally a stack trace, nothing from the data being analysed.

- **Parsed log entries or line contents.** Logs contain app data and may contain user
  identifiers; and a 200MB import would emit millions of lines.
- **HTTP headers, cookies, tokens, request/response bodies.** Never print these to stderr —
  the payload may hold real credentials. `network::redact` can replace secrets before the data
  reaches the UI, but `REDACT_ENABLED` is off by default (raw capture kept for debugging), so
  assume the UI/session/export contain plaintext. `redacted: true` only appears when the gate
  is enabled.
- **Per-poll / per-keystroke diagnostics.** The stream poll runs every 800ms; a log line here
  becomes noise immediately. The one exception is the *folded* flow snapshot above: one
  record per boundary/session, refreshed in place, never one line per request or row.
- **URLs, query strings or payloads in a flow checkpoint.** Counters and status strings only —
  this file gets pasted into bug reports.
- **File paths the user did not choose**, device serials beyond what is needed, or anything
  that would be surprising to see in a bug report.
- **Errors that are already returned to the UI** — log only when the UI cannot show them
  (shutdown, background thread).

---

## Common Mistakes

1. **Using `println!` instead of `eprintln!`** — stdout is the app's normal output channel;
   diagnostics belong on stderr.
2. **Logging a request body to debug redaction** — that is exactly the data redaction exists
   to protect.
3. **Adding `log` + `env_logger` for one `debug!` call** — the dependency is not worth it.
4. **Silencing a failure with `let _ =` and no log** — cleanup failures are the one place a
   log line is required.
5. **Logging inside the parser hot loop** — use counters (`unparsed_lines`, skipped lines) and
   log a summary at the boundary.
