# Error Handling

> Errors cross exactly one boundary: `Result<T, String>` returned from a Tauri command, shown
> verbatim by the UI.

---

## Overview

- No custom error enum or `thiserror`. Most commands return `Result<T, String>`. Two read-only
  probes return plain data instead: `adb_status() -> adb::AdbStatus` and
  `list_builtin_templates() -> Vec<Rule>`; the `ipc` wrappers on the frontend mirror that.
- The string is **user-facing Chinese** and includes the failed action plus the underlying
  cause: `format!("读取规则失败: {e}")`.
- Errors are returned to the frontend, where `errorMessage(e)` normalizes them and the store
  puts them in `store.error` or a local `error` ref.
- Background threads cannot return a `Result`; they store the failure in shared state and
  surface it on the next poll (`StreamStatus.error`).
- Nothing panics on expected failures (missing adb, unauthorized device, cancelled import,
  malformed JSONL line).

---

## Error Types

There is one type: `String`. It plays three roles:

| Role | Example | Displayed how |
|------|---------|---------------|
| Validation / precondition | `"所选路径不是文件"` | Error banner / inline error |
| IO or process failure | `format!("无法启动 adb logcat: {e}")` | Error banner |
| Cancellation | `"已取消导入"` | Import card closes; treated as a normal outcome |

State probes that are not failures return data instead of an error: `adb_status()` returns
`AdbStatus { available: false, error: "..." }` when adb is missing, because "no adb" is a
normal state for file-only users.

---

## Error Handling Patterns

### Map at the boundary, keep context

```rust
let meta = fs::metadata(&file_path).map_err(|e| format!("无法读取文件信息: {e}"))?;
let text = fs::read_to_string(&path).map_err(|e| format!("读取规则失败: {e}"))?;
```

- Use `?` for propagation once the message is meaningful.
- Do not let a bare `std::io::Error` string reach the UI; it is English and context-free.
- Do not lose the cause: keep `{e}` in the message.

### Locks

- In command paths, a poisoned lock is a real error:
  `.lock().map_err(|e| format!("状态锁异常: {e}"))?`
- In cleanup paths, a poisoned lock must **not** stop cleanup:
  `.lock().unwrap_or_else(|p| p.into_inner())` (`AppState::stop_all_streams`). Leaving a
  device pointing at a dead proxy is worse than a panicked worker.

### Worker threads

- `stream.rs` stores `error`, `stderr` and `finished` in `StreamBuffer`.
- `drain()` converts a non-success child exit into a message, preferring captured stderr over
  an exit code: `"adb logcat 异常退出（退出码 {code}）"`.
- `startup_error()` gives adb ~200ms to fail fast (8 x 25ms) so an unauthorized device is
  reported as an error instead of "connected but empty".
- A reader-thread error flushes whatever was parsed before reporting, so partial data is not
  lost.

### Cancellation

- `open_log_file` registers an `Arc<AtomicBool>` **before** parsing starts.
- The parse callback returns `false` to stop; `ParseOutcome::Cancelled` becomes
  `Err("已取消导入")`.
- A cancel that arrives after the last chunk still wins (checked again before registering the
  session), so clicking 取消 at 99% cannot open a tab.

---

## API Error Responses

Tauri turns `Err(String)` into a rejected promise. The frontend receives the raw string and
normalizes it with `errorMessage`:

```ts
try {
  const result = await ipc.queryLogs(...)
} catch (e) {
  store.error = errorMessage(e)
}
```

Consequences for backend authors:

- The string is the entire UI message. Make it actionable and Chinese.
- Do not include stack traces, file paths the user did not choose, or secrets.
- Do not return `Ok` with a sentinel error field for a genuine failure; use `Err`.
- Partial success is represented explicitly in the payload (`StreamStatus.error`,
  `ProbeReport.healedProxy`, skipped-line counters), not by throwing away good data.

---

## What Must Never Happen

| Pattern | Why |
|---------|-----|
| `unwrap()` / `expect()` on user input, file IO, locks in a command | Crashes the process instead of showing a message |
| `panic!` for an expected condition (no device, bad path) | Same |
| Returning an English `io::Error` string directly | Violates the user-facing message rule |
| Swallowing an error with `let _ =` when the user needs to know | Silent failure; only acceptable for idempotent cleanup |
| Logging an error and also returning an empty `Ok` | The UI shows success while nothing happened |
| Holding a lock while formatting/writing a large response | Blocks every other session operation |

---

## Common Mistakes

1. **Forgetting `?` after `map_err`** — the error is constructed but ignored.
2. **Using `unwrap()` on `serde_json::to_string` for wire payloads** — serialization should be
   handled with a real error path; `Default`-derived types make this rare.
3. **Treating "no rule file yet" as an error** — it must return an empty default file.
4. **Reporting a cancel as a crash** — cancellation is a normal `Err("已取消导入")`.
5. **Losing stderr** — adb failures are usually explained only on stderr; always prefer it
   over a bare exit code.
