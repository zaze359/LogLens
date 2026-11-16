# LogAnalyse Spec Index

> Entry point for `.trellis/spec/`. Read this first, then follow the links for the layer you
> are changing.

LogAnalyse is a Tauri 2 desktop app: a Vue 3 + TypeScript UI in `src/` talking over IPC to a
Rust process in `src-tauri/`. The specs describe **this codebase as it exists**, not an ideal
architecture. When code and spec disagree, the code wins — then update the spec.

---

## Directory Map

| Spec directory | Maps to | Covers |
|----------------|---------|--------|
| [`frontend/`](./frontend/index.md) | `src/` | Vue SFC conventions, directory layout, store/state model, watchers/timers, type safety, frontend quality gates |
| [`backend/`](./backend/index.md) | `src-tauri/` | Rust module layout, command/IPC wiring, in-memory + `rules.json` persistence, error strings, process/thread lifetime, logging |
| [`guides/`](./guides/index.md) | Both layers | Cross-layer thinking and reuse thinking: boundaries, contract ownership, intentional duplication |
| [`pending-code-fixes.md`](./pending-code-fixes.md) | Both layers | Fix record for spec-vs-code gaps (all four items fixed on 2026-11-16) |

The directory names are historical: `backend` means the **native Rust side**, not an API
server. There is no API backend, database or worker service; `proxy.rs` does run a local
HTTP/CONNECT capture proxy listener.

---

## Read By Scenario

### I am adding or changing UI

| Task | Read |
|------|------|
| New component / dialog / pane | [frontend/component-guidelines.md](./frontend/component-guidelines.md), [frontend/directory-structure.md](./frontend/directory-structure.md) |
| New shared state, tab behavior, polling | [frontend/state-management.md](./frontend/state-management.md) |
| Watchers, timers, Tauri event listeners | [frontend/hook-guidelines.md](./frontend/hook-guidelines.md) |
| New IPC payload or command wrapper | [frontend/type-safety.md](./frontend/type-safety.md), [guides/cross-layer-thinking-guide.md](./guides/cross-layer-thinking-guide.md) |
| Styling a new region | [frontend/component-guidelines.md](./frontend/component-guidelines.md) (global `styles/main.css` only) |
| Preparing a frontend handoff | [frontend/quality-guidelines.md](./frontend/quality-guidelines.md) |

### I am adding or changing native behavior

| Task | Read |
|------|----------|
| New Tauri command | [backend/directory-structure.md](./backend/directory-structure.md) ("Adding a command"), [backend/error-handling.md](./backend/error-handling.md) |
| New module or refactor | [backend/directory-structure.md](./backend/directory-structure.md) |
| Log parsing / filtering / rule semantics | [backend/directory-structure.md](./backend/directory-structure.md), [backend/quality-guidelines.md](./backend/quality-guidelines.md), [guides/code-reuse-thinking-guide.md](./guides/code-reuse-thinking-guide.md) |
| Device capture, process or thread lifecycle | [backend/quality-guidelines.md](./backend/quality-guidelines.md), [backend/error-handling.md](./backend/error-handling.md) |
| Persistence / `rules.json` format | [backend/database-guidelines.md](./backend/database-guidelines.md) |
| Diagnostics and what must not be logged | [backend/logging-guidelines.md](./backend/logging-guidelines.md) |
| Preparing a backend handoff | [backend/quality-guidelines.md](./backend/quality-guidelines.md) |

### I am changing the wire contract (both layers)

Read all three in this order:

1. [backend/directory-structure.md](./backend/directory-structure.md) — module and command wiring
2. [frontend/type-safety.md](./frontend/type-safety.md) — the `model.rs` ↔ `types.ts` mirror
3. [guides/cross-layer-thinking-guide.md](./guides/cross-layer-thinking-guide.md) — flow, owners, checklists

A wire change is not done until the Rust definition (`src-tauri/src/model.rs` or the owning
domain module such as `adb.rs` / `network.rs`) and its mirror in `src/types.ts` are updated,
the command is registered in `lib.rs`, and a wrapper exists in `src/lib/ipc.ts`.

### I am fixing a bug or reviewing code

- Start with [guides/cross-layer-thinking-guide.md](./guides/cross-layer-thinking-guide.md) if
  the bug crosses UI ↔ Rust, or involves `find`/`filter`, paging, polling or device cleanup.
- Use [guides/code-reuse-thinking-guide.md](./guides/code-reuse-thinking-guide.md) before
  extracting a helper or changing a duplicated constant.
- Use the layer-specific quality checklist for the final pass.

### I am new to the project

1. [frontend/index.md](./frontend/index.md) and [backend/index.md](./backend/index.md) for the
   shape of each side.
2. [frontend/directory-structure.md](./frontend/directory-structure.md) and
   [backend/directory-structure.md](./backend/directory-structure.md) for where things live.
3. [guides/index.md](./guides/index.md) for the thinking triggers.

---

## The Contract In One Screen

```
src/ (Vue)                         src-tauri/ (Rust)
─────────────                      ─────────────────
types.ts          <── camelCase ──  model.rs + domain modules
lib/ipc.ts        ── invoke ──────> commands.rs ──> session.rs / logcat.rs / rules.rs
stores/session.ts <── Result<T> ───  (errors are Chinese strings)
components/*.vue                   AppState (Mutex maps) + rules.json
```

Facts that are easy to get wrong:

- **Rust owns data.** Entries, filtering, paging, rule matching and persistence happen in
  `src-tauri`; the UI renders results and keeps per-tab view snapshots.
- **`find` is not `filter`.** `FilterSpec.searchMode` decides whether the search term narrows
  the result set. Export and paging always follow `filter`.
- **Wire defaults are not optional.** Rust uses `#[serde(default)]`; TS interfaces are
  non-optional. Add fields on both sides in the same change.
- **Limits are mirrored where they exist.** `MAX_OPEN_SESSIONS = 4` (Rust) ↔ `MAX_TABS = 4`
  (UI). The UI asks for 400 entries per page; Rust has no page-size constant and accepts
  `limit` from the caller.
- **Device state outlives the process.** Proxy/forward cleanup must be reachable from every
  shutdown path, and startup self-heals stale proxies.
- **No new layers casually.** No router, Pinia, CSS framework, database, logging crate or
  second IPC wrapper. The existing patterns are deliberate.

---

## Maintenance Rules

- Update the owning spec in the same change as the code; do not leave a spec describing
  behavior that no longer exists.
- Every index (`index.md` in this directory and in each subdirectory) must list the real file
  set. Adding/removing a spec file requires updating the index.
- No placeholders, empty headings or copied boilerplate. Prefer a concrete file path and a
  real example.
- If a template section does not apply to this project (e.g. SQL migrations), replace it with
  what the project actually does instead of leaving it empty.
- Open spec-vs-code gaps live in [pending-code-fixes.md](./pending-code-fixes.md). When one is
  fixed, mark it fixed there, keep the record, and remove the gap reference from the owning
  spec.
