# Quality Guidelines

> What "done" means for a Rust change in this repo.

---

## Overview

- No `clippy.toml` or lint deny-list; the baseline is `cargo test --lib` plus a clean
  `cargo build`.
- Formatting follows `cargo fmt` (4 spaces, standard Rust style).
- Tests live beside the code: `#[cfg(test)] mod tests` at the bottom of each module.
  ~98 `#[test]` functions exist today; 10 real-device tests are `#[ignore]`d across
  `adb.rs`, `netcapture.rs`, `netprobe.rs`, `proxy.rs` and `stream.rs`.
- Correctness matters more than abstraction: parsing, filter semantics and process cleanup are
  covered; UI glue is not unit-tested.

---

## Verification Commands

```bash
cd src-tauri
cargo test --lib                                   # pure logic, no device required
cargo test --lib -- --ignored                      # real adb device; opt-in
cargo fmt --check                                  # formatting
cargo build                                        # compile check
cd .. && npm run build                             # frontend type check + bundle
```

---

## Forbidden Patterns

| Pattern | Why |
|---------|-----|
| Blocking the IPC thread with file/process/socket work | Freezes the window; use `async` + `spawn_blocking` or a reader thread |
| `unwrap()` / `expect()` on user input, file IO or locks in a command | Must return `Err(String)` instead; panics kill the desktop process. `.expect("valid ...")` on static regexes / compile-time invariants is accepted |
| Spawning a child process without an owner that kills it | A leaked `adb logcat` or proxy keeps running after the window closes |
| Holding a `MutexGuard` across `await`, IO, or another lock | Deadlocks the poll loop and the UI |
| Unbounded collections driven by user/device input | Logs can be 200MB; cap caches, hit lists and probe outputs |
| Re-parsing the whole file on every keystroke | The match cache and debounce exist for this reason |
| Adding `tokio`/`async-std`/`tracing` for one feature | Tauri already provides a runtime; the app uses std threads + `eprintln!` |
| Ad-hoc `serde_json::json!` for wire payloads | Wire shapes belong in `model.rs`; `json!` hides field drift |
| Changing a command name without updating `lib.rs` and `ipc.ts` | Runtime "command not found", no compile-time signal |
| Using `Path::new`/file IO inside `model.rs` | `model.rs` is types only |

---

## Required Patterns

### Resource ownership

- A struct that owns a child process or a device-side change must implement teardown:
  `DeviceStream::stop` (idempotent) and `ProxyGuard::drop` (restores the proxy).
- `AppState::stop_all_streams` is called from `WindowEvent::CloseRequested`,
  `WindowEvent::Destroyed`, `RunEvent::Exit` **and** `Drop`. Do not add a new shutdown path
  that skips it.
- Startup calls `netcapture::heal_stale_proxies()` so a crash that skipped every hook cannot
  leave the phone offline.

### Threads and polling

- Reader threads own their `StreamBuffer` through `Arc<Mutex<_>>`; they never touch
  `AppState` or Tauri types.
- A poll must be safe to call repeatedly: `drain()` takes pending entries and reports status;
  `stop()` on an exited child is a no-op.
- Re-entrancy in the UI is handled by the frontend (`polling` flag), but backend polls should
  remain cheap enough that overlap is harmless.

### Bounds

| Bound | Value | Location |
|-------|-------|----------|
| Open log sessions | `MAX_OPEN_SESSIONS = 4` | `commands.rs` |
| Cached match lists per session | `MAX_CACHED_QUERIES = 12` | `session.rs` |
| Import progress events | 1 per 100ms | `commands.rs` |
| `find` hit indices | caller-supplied `limit` (5000 from the UI) | `session.rs::find` |

Every new unbounded list must get a cap or a documented reason.

### Parsing

- Parsers are pure: `logcat::LogcatParser`, `adb::parse_devices`, `network` HAR/JSONL parsing.
  They take bytes/text and return values; no global state, no Tauri.
- Tolerate dirty input: invalid UTF-8 is decoded lossily (`from_utf8_lossy`), malformed JSONL
  lines are skipped with a counter/warning, unknown fields are ignored (`serde(default)`).
- Keep continuation lines with their entry (`message`, `continuation`) and keep `raw`
  loss-free for export.

### Wire types

- Shared core IPC payloads live in `model.rs`; domain payloads live beside their module
  (`adb.rs`, `network.rs`, `netcapture.rs`, `netprobe.rs`, `config_store.rs`). All of them derive
  `Debug, Clone, Serialize, Deserialize, Default` + `#[serde(rename_all = "camelCase", default)]`.
- Internal-only types (`LogSession`, `AppState`, `DeviceStream`) do not derive serde.

---

## Testing Requirements

- New pure logic (parser branch, filter operator, normalisation rule, arg builder) needs a
  `#[test]` in the same module.
- Test names state the behavior, not the method:
  `find_reports_positions_without_narrowing_the_list`,
  `parses_v1_document_and_bare_array`.
- Assertion messages may be Chinese (they explain the invariant to the reader).
- Tests must not require a device or network. If they do, mark them
  `#[ignore = "需要本机安装 adb 并连接设备"]` and document the run command.
- Do not write tests that only restate a constant or that pass with the feature deleted.

---

## Code Review Checklist

- [ ] Command registered in `lib.rs` and wrapped in `src/lib/ipc.ts`
- [ ] Wire struct in `model.rs` and TS mirror in `types.ts` changed together
- [ ] Error path returns Chinese `Err(String)`, not a panic
- [ ] Locks are short-lived and not held across IO/await
- [ ] Every spawned process has a stop/kill path reachable from shutdown
- [ ] New collection has a bound
- [ ] Appending/rebuilding invalidates the match cache
- [ ] `cargo test --lib` passes; `cargo fmt --check` is clean
- [ ] No new dependency without a concrete need
