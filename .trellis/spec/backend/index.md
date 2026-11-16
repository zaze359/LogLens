# Backend Development Guidelines

> Conventions for `src-tauri/` — the Rust process behind the Vue UI.

---

## Overview

| Item | Reality in this repo |
|------|----------------------|
| Language | Rust 2021, `rust-version = "1.90"` |
| Framework | Tauri 2 (`tauri`, `tauri-plugin-dialog`) |
| Crates | `serde`, `serde_json`, `regex`. No direct `tokio`, `tracing` or `log` dependency |
| Shape | A single desktop process. No API backend, database or worker queue; `proxy.rs` runs a local HTTP/CONNECT capture proxy listener |
| IPC | `#[tauri::command]` functions registered in `src-tauri/src/lib.rs` |
| State | `AppState` (managed by Tauri) with `Mutex<HashMap<...>>` maps |
| Persistence | `rules.json` in the OS app-config directory (`config_store.rs`); built-in templates embedded at compile time |
| Tests | `#[cfg(test)] mod tests` next to each module; `cargo test --lib` |

"Backend" here means the **native side of the desktop app**. It owns everything the webview
cannot safely do: file IO, process spawning, adb, sockets, a local HTTP proxy, parsing and
filtering large logs.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module map, dependency direction, adding a command | Ready |
| [Database Guidelines](./database-guidelines.md) | No DB: in-memory sessions + versioned `rules.json` | Ready |
| [Error Handling](./error-handling.md) | `Result<_, String>`, user-facing Chinese messages, poisoned locks | Ready |
| [Quality Guidelines](./quality-guidelines.md) | Tests, process/thread lifetime, bounds, forbidden patterns | Ready |
| [Logging Guidelines](./logging-guidelines.md) | `eprintln!` conventions, redaction, hot-path silence | Ready |

---

## Commands

```bash
npm run tauri dev                    # full desktop app
cd src-tauri && cargo test --lib     # unit tests (no device required)
cd src-tauri && cargo test --lib -- --ignored   # real-device probes, skipped by default
npm run build                        # frontend type check + bundle (also run before handoff)
```

`cargo test` is run from `src-tauri/`, not the repo root.

---

**Language**: Specs are written in English. Error strings, comments and log text are in
Chinese, matching the existing code.
