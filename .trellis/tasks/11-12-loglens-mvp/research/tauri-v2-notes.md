# Research: Tauri v2 APIs used by this project

Sources: https://v2.tauri.app/plugin/dialog/ , https://v2.tauri.app/reference/config/ ,
https://v2.tauri.app/develop/calling-rust/ (fetched 2026-11-12).

## Confirmed facts

- `tauri-plugin-dialog` v2 **requires Rust >= 1.90**. `Cargo.toml` therefore declares
  `rust-version = "1.90"`.
- JS package: `@tauri-apps/plugin-dialog`; `open({ multiple, directory, title })` and
  `save({ title, defaultPath, filters })`. `defaultPath` is a valid option.
- Rust init: `tauri_plugin_dialog::init()`; permissions: `dialog:allow-open`,
  `dialog:allow-save` (the `dialog:default` set already includes both, but explicit
  grants are clearer). `core:default` is valid in capabilities.
- `tauri.conf.json` v2: `build.beforeDevCommand / devUrl / beforeBuildCommand /
  frontendDist`, `app.windows[]` with `label`, `bundle.active`, `bundle.icon` (default
  `[]`, optional). `identifier` must be a reverse-domain id and must not be
  `com.tauri.dev`.
- IPC arguments: JS uses **camelCase** keys, Rust command parameters are
  **snake_case**; Tauri maps them automatically (`destPath` → `dest_path`,
  `sessionId` → `session_id`).
- `getCurrentWebview().onDragDropEvent(cb)` (from `@tauri-apps/api/webview`) receives
  `{ payload: { type: 'drop', paths: string[], position } }` — this is how native drag
  & drop yields absolute paths, which HTML5 `File` objects cannot provide.
- `app.path().app_config_dir()` (requires `use tauri::Manager`) resolves to
  `~/Library/Application Support/<identifier>` on macOS and `%APPDATA%/<identifier>`
  on Windows.

## Implications for this project

- All file IO lives in Rust commands; no `fs` plugin and no scope configuration needed,
  even for arbitrary user-selected paths.
- Bundling needs real icon files. Keep `bundle.icon: []` for a working `tauri dev` on a
  fresh clone; run `npm run tauri icon` and fill the icon array before `tauri build`.
- Cross-platform release requires building on each target OS (or CI matrices); Tauri
  cannot produce a Windows MSI from macOS.
