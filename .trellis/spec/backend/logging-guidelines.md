# Logging Guidelines

> There is no logging crate. The app writes diagnostics to stderr with `eprintln!`, and
> user-facing problems travel through `Result` instead.

---

## Overview

- No `log`, `tracing` or `env_logger`. `Cargo.toml` intentionally has no logging dependency.
- `eprintln!` goes to the terminal that launched `npm run tauri dev`; in a bundled app it is
  usually invisible. Therefore **stderr is for developers**, not for users.
- Anything the user must see is returned from a command and rendered by the UI.
- Logging is sparse: startup/cleanup, skipped malformed input, and background-thread failures.
  There is no per-entry or per-request logging.

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
