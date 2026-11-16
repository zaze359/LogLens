# Frontend Development Guidelines

> Conventions for `src/` — a Vue 3 + TypeScript single-page desktop UI rendered inside Tauri 2.

---

## Overview

| Item | Reality in this repo |
|------|----------------------|
| Framework | Vue 3.5 SFC, `<script setup lang="ts">` only |
| Build | Vite 6 (`vite.config.ts`), fixed dev port `1420` |
| Types | TypeScript strict (`tsconfig.json`), checked by `vue-tsc --noEmit` |
| State | One module-level `reactive` store in `src/stores/session.ts`. No Pinia, no Vuex |
| Routing | None. Workspaces are switched with `v-show` / store flags |
| Styling | One global stylesheet `src/styles/main.css`; no `<style>` blocks. Some layout/size values use inline `style` attributes |
| Native access | All Rust IPC through `src/lib/ipc.ts`. Components do not call `invoke` directly (one documented exception: `NetworkCaptureTab.vue`) |
| Tests | No frontend test runner. Verification is `npm run build` (type check + bundle) |

The frontend is a **view layer over Rust-owned state**. Parsing, filtering, paging, rule
matching and persistence all happen in `src-tauri`. The UI keeps per-tab view snapshots and
polls live captures; it must not re-implement backend logic.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module organization and file layout | Ready |
| [Component Guidelines](./component-guidelines.md) | SFC structure, props/emits, styling, pane lifetime | Ready |
| [Hook Guidelines](./hook-guidelines.md) | Composition functions, watchers, timers, event subscriptions | Ready |
| [State Management](./state-management.md) | Singleton store, tab snapshots, request races, polling | Ready |
| [Quality Guidelines](./quality-guidelines.md) | Verification commands, forbidden patterns, review checklist | Ready |
| [Type Safety](./type-safety.md) | IPC type mirror, `unknown` handling, `plain()`, casts | Ready |

---

## Cross-layer companion

`src/types.ts` mirrors the Rust wire types: core payloads from `src-tauri/src/model.rs`, plus
domain payloads defined in `adb.rs`, `network.rs`, `netcapture.rs`, `netprobe.rs` and
`config_store.rs`. Before changing a wire type, read
[../backend/directory-structure.md](../backend/directory-structure.md) and
[../guides/cross-layer-thinking-guide.md](../guides/cross-layer-thinking-guide.md).

---

**Language**: Specs are written in English. UI copy, comments and user-facing error strings
are Chinese, matching the existing codebase.
