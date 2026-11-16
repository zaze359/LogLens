# Directory Structure

> How `src-tauri/src/` is organized.

---

## Directory Layout

```
src-tauri/
├── Cargo.toml
├── tauri.conf.json
├── templates.json           # Built-in rules/categories, embedded with include_str!
└── src/
    ├── main.rs              # Entry point: log_analyse_lib::run()
    ├── lib.rs               # Tauri builder, AppState, command registration, shutdown hooks
    ├── commands.rs          # Every #[tauri::command]; session lifecycle and orchestration
    ├── model.rs             # Wire types shared with the frontend (serde, camelCase)
    ├── session.rs           # LogSession (entries + match cache), NetworkSession, AppState
    ├── stream.rs            # Live `adb logcat` child process, reader threads, incremental parse
    ├── logcat.rs            # Logcat parser, continuation merging, facets, pid->package
    ├── adb.rs               # adb discovery, command/arg builders, device parsing, probes
    ├── rules.rs             # Condition compilation, text matcher, quick filters, matching
    ├── config_store.rs      # rules.json load/save/normalise, builtin/user merge
    ├── builtin.rs           # Embedded templates + built-in categories
    ├── netprobe.rs          # Device capability probe and capture-route recommendation
    ├── netcapture.rs        # Capture abstraction, proxy journal, stale-proxy self-heal
    ├── network.rs           # NetworkRequest, HAR/JSONL parsing, secret redaction
    ├── cdp.rs               # WebView DevTools Protocol capture
    └── proxy.rs             # Built-in HTTP/CONNECT proxy capture
```

---

## Module Organization

### Responsibility map

| Module | Owns | Must not own |
|--------|------|--------------|
| `lib.rs` | Wiring: builder, managed state, handler list, window/exit cleanup | Business logic |
| `commands.rs` | IPC boundary, validation of command args, session registration/eviction, orchestration between modules | Parsing algorithms, adb arg details |
| `model.rs` | Wire structs only | Behavior |
| `session.rs` | Parsed entries, match cache, `AppState` maps, shutdown | Parsing, IPC concerns |
| `stream.rs` | Child process + reader threads + `StreamBuffer` | Tauri types, session locking |
| `logcat.rs` / `adb.rs` | Pure parsing / pure process helpers; testable without a device | UI state |
| `rules.rs` | Filter compilation and matching | Persistence |
| `config_store.rs` | File persistence and normalisation | Rule semantics |
| `netprobe.rs` / `netcapture.rs` / `cdp.rs` / `proxy.rs` / `network.rs` | Device network capture pipeline and redaction | Log sessions |

### Dependency direction

```
lib.rs -> commands.rs -> { session, stream, logcat, adb, rules, config_store, builtin,
                           netcapture, netprobe, network, cdp, proxy }
model.rs <- everyone (types only)
```

- Domain modules do not import `commands.rs`.
- Command paths lock `AppState` in `commands.rs`. The shutdown path also locks it in
  `session.rs::stop_all_streams`, called from `lib.rs` on window close/exit and from `Drop`.
  Domain modules never lock `AppState`.
- Modules that need Tauri (`config_store` needs `AppHandle` for the config path) take it as a
  parameter; do not reach for a global.
- There is no `utils.rs`. A helper lives in the module that owns the domain; it becomes a new
  module only when it has its own state or process/lifecycle.

### Adding a command

1. Add the wire types to `model.rs` with `#[serde(rename_all = "camelCase", default)]`.
2. Implement `#[tauri::command] pub fn/async fn ... -> Result<T, String>` in `commands.rs`.
3. Register it in `invoke_handler![...]` in `lib.rs`.
4. Mirror the type in `src/types.ts` and add a wrapper in `src/lib/ipc.ts`.
5. Add unit tests for any pure logic in the owning module.

Forgetting step 3 produces "command not found" at runtime, with no compile error on either
side — this is the most common wiring mistake.

### Long-running work

- File import: `async fn open_log_file` + `spawn_blocking`, progress throttled to one event
  per `IMPORT_PROGRESS_MS = 100`, cancellation via `Arc<AtomicBool>`.
- Live log capture: `std::process::Child` + two reader threads, drained by `poll_device_stream`.
- Network capture: capture object stored in `AppState.network_captures`, polled by the UI.
- Do not block the Tauri IPC thread with file/process/socket work.

---

## Naming Conventions

| Kind | Rule | Example |
|------|------|---------|
| Module file | snake_case, one domain per file | `netcapture.rs` |
| Command function | snake_case verb phrase; the name is the IPC command string | `open_log_file`, `poll_device_stream` |
| Wire struct field | snake_case in Rust, camelCase on the wire | `search_mode` -> `searchMode` |
| Internal struct | PascalCase noun | `DeviceStream`, `CompiledFilter` |
| Constants | UPPER_SNAKE_CASE | `MAX_OPEN_SESSIONS`, `IMPORT_PROGRESS_MS` |
| Error string | Chinese, sentence-like, includes the failing action and the cause | `format!("读取规则失败: {e}")` |
| Test module | `#[cfg(test)] mod tests` at the bottom of the file | `session.rs` |

---

## Examples

- Command orchestration and cancellation: `commands.rs` (`open_log_file`, `cancel_open_log`)
- Process/thread ownership: `stream.rs` (`DeviceStream::spawn`, `drain`, `stop`)
- Cached query model: `session.rs` (`matched_indices`, `query`, `find`)
- Versioned persistence: `config_store.rs` (`parse_document`, `normalise`, `save`)
- Resource guards + shutdown: `session.rs` (`AppState::stop_all_streams`, `Drop`) and
  `netcapture.rs` (`ProxyGuard`, `heal_stale_proxies`)
