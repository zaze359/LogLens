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

### Client error log (`error_log.rs`)

- File: `<app_config_dir>/error-log.jsonl` — the same directory as `rules.json`.
- Records **only** this app's own failures: Rust panics (global hook with backtrace),
  uncaught frontend errors (window/Vue/rejection handlers → `report_client_error`), and
  Tauri commands that returned `Err` (captured by the `call()` wrapper in `ipc.ts`).
- Bounded: `MAX_RECORDS = 500`, newest kept; identical failures inside 10s fold into one
  record with an incrementing `count`. This is what makes per-poll failures safe to include.
- Deliberately excludes `poll_device_stream` / `poll_network_capture` (`SILENT_COMMANDS` in
  `ipc.ts`): teardown makes them fail routinely, and their real errors already reach the UI
  through `StreamStatus.error` / capture status.
- The UI in `ErrorLogPanel.vue` lists / filters / exports / clears it. It is user-visible
  support data, so export is plain text (not JSONL) and the path is shown in the panel.
- Initialising the log or recording into it must never panic or abort startup — a poisoned
  mutex is recovered with `into_inner()`, and init failures fall back to stderr.

---

## Log Levels

There are no levels; use the message to convey severity.

| Situation | Prefix / wording | Example |
|-----------|------------------|---------|
| Startup self-heal | `[proxy]` | `eprintln!("[proxy] 启动自愈: {healed}")` in `lib.rs` |
| Cleanup failed but the app continues | action + reason | `eprintln!("恢复设备代理失败: {e}")` in `proxy.rs` |
| Skipped malformed external input | what + where | `eprintln!("跳过第 {} 行 JSONL: {e}", line_no + 1)` |
| Test/helper diagnostics | free-form | `eprintln!("记录到 {} 条请求...", rows.len())` |

Rules:

- Keep one-line, greppable messages. Include the failing action and the cause.
- A prefix in brackets is used only for the startup proxy heal today (`[proxy]`); do not
  invent a prefix taxonomy.
- Failure of a *user-initiated* action is returned as `Err`, not logged and dropped.

---

## Structured Logging

There is none. Messages are human-readable strings. If a structured log ever becomes
necessary, prefer adding `serde_json` output to the same `eprintln!` rather than introducing
a logging framework for one call site.

---

## What to Log

- Startup self-healing results (`heal_stale_proxies`).
- Device-side cleanup failures that cannot be reported to the UI because the window is going
  away (proxy restore, guard drop).
- Malformed lines in external capture files (JSONL/HAR) so a user can diagnose a truncated
  export. These are skips, not failures.
- Reader-thread errors after the associated data has been flushed.

## What NOT to Log

The rules below apply to **both** stderr and the client error log. The error log additionally
never stores parsed entries, request bodies or file contents — it stores an error string,
a scope and a stack trace, nothing from the data being analysed.

- **Parsed log entries or line contents.** Logs contain app data and may contain user
  identifiers; and a 200MB import would emit millions of lines.
- **HTTP headers, cookies, tokens, request/response bodies.** Network capture redacts secrets
  in `network::redact` before the data reaches the UI; do not undo that by logging raw
  values. `redacted: true` means the row was modified.
- **Per-poll / per-keystroke diagnostics.** The stream poll runs every 800ms; a log line here
  becomes noise immediately.
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
