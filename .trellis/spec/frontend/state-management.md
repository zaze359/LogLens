# State Management

> One reactive store, per-tab snapshots, and explicit race guards. There is no server cache
> library: the Rust process *is* the data source.

---

## Overview

`src/stores/session.ts` exports:

```ts
export const store = reactive<AppState>({ ... })
```

and a set of functions that mutate it. Components import `store` directly and call the
functions. This is deliberate: one app, one window, no routing, no SSR, no multi-user state.

The store is **not** a cache of the backend. Rust owns entries, filters, rules and sessions.
The store owns:

- which log sessions are open (`tabs`)
- the active tab's view state (a flat mirror used by every component)
- UI-only concerns: selection, scroll offset, import card, toasts, device workspace flags
- polling timers for live captures

---

## State Categories

| Category | Where it lives | Examples |
|----------|----------------|----------|
| Global app state | `store` | `tabs`, `activeId`, `devices`, `deviceWorkspaces`, `rules`, `categories` |
| Active-tab mirror | flat fields on `store` | `fileInfo`, `filter`, `entries`, `result`, `selected` |
| Per-tab snapshot | `SessionTab` | `filter`, `entries`, `result`, `selected`, `scrollTop`, `stream` |
| Ephemeral UI state | local `ref` in a component | `NetworkCaptureTab` probe report, dialog open state, copied-curl flash |
| Backend-owned state | Rust `AppState` | parsed entries, match cache, live streams, proxy guards |
| Persisted state | `rules.json` via `config_store.rs` | user rules and categories (never builtin ones) |

---

## The Two-Tier Tab Model

Each open log owns its own backend session and its own view state. Only one tab's state is
live in the flat mirror at a time.

```ts
// Leaving a tab: copy the live view into that tab.
function snapshotActive(): void {
  tab.filter = clone(store.filter)
  tab.entries = store.entries
  tab.scrollTop = store.scrollTop
  tab.stream = store.deviceStream
}

// Entering a tab: restore it, then set `activeId` last.
store.filter = clone(target.filter)
store.entries = target.entries
store.scrollTop = target.scrollTop
store.activeId = id
```

Rules:

1. `filter` must be deep-cloned (`clone` = JSON round-trip). `updateFilter` mutates
   `store.filter` in place; two tabs must never share the object.
2. Set `store.activeId` **last**, because `LogTable` watches it to restore scroll.
3. Background tabs keep receiving lines in their Rust session but are not re-rendered.
   `pollInBackground` / `catchUp` pull deltas when the tab becomes active.
4. Closing a tab releases the backend session (`ipc.closeSession`) even though the tab is
   removed from the strip first, so the UI reacts instantly.

---

## Request Races

Every asynchronous result that can mutate shared view state is guarded by a monotonic token.

```ts
const seq = ++requestSeq
const result = await ipc.queryLogs(...)
if (seq !== requestSeq) return   // an older filter already lost
store.result = result
```

- `requestSeq` guards queries and paging. `loadMore` additionally checks that the entry count
  has not changed since the request started.
- `findSeq` guards search-hit requests (`runFind`).
- Switching tabs increments `requestSeq`, making in-flight responses from the previous tab
  unlandable.
- Both `pollInBackground` and the visible tab's `syncStream` use the same per-tab `polling`
  boolean, so a slow poll cannot stack another one.

Do not add an async store mutation without one of these guards.

---

## Debounce and Poll Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `PAGE_SIZE` | 400 | UI query page size; Rust has no matching constant and accepts `limit` from the caller |
| `MAX_TABS` | 4 | Must equal Rust `MAX_OPEN_SESSIONS` (`commands.rs`) |
| `STREAM_POLL_MS` | 800 | Live log / network capture poll cadence |
| `IMPORT_CARD_DELAY_MS` | 250 | Do not flash the import card for fast files |
| refresh debounce | 220ms | `scheduleRefresh` |
| find debounce | 250ms | `scheduleFind` |

`MAX_TABS` is a cross-layer constant: if Rust starts evicting the oldest session at a
different count, the UI will show a tab whose queries all fail.

---

## Global vs. Local State

Promote to the store when **two or more components** need it or when it must survive pane
switching. Keep it local otherwise.

- Device tool selection (`deviceOps`) is global because the rail highlights it and the content
  area switches on it.
- Network capture results stay local to `NetworkCaptureTab` because nothing else renders them.
- `selected` log entry is global because both the table and the detail panel read it.

---

## Common Mistakes

1. **Destructuring the store** — `const { entries } = store` loses reactivity. Read
   `store.entries` in templates/computed.
2. **Mutating `store.filter` from a component** — use `updateFilter` / `updateSearch` so
   refresh and find stay consistent.
3. **Writing an async action without a sequence token** — a slow response overwrites a newer
   filter, or a response from tab A lands in tab B.
4. **Adding a new global event/timer without `syncTimers()`** — the poll loop must be started
   and stopped from one place.
5. **Treating `result` as complete** — `entries` is paged (`PAGE_SIZE`); `result.matched` is
   the total match count, `result.total` is the file's entry count.
6. **Forgetting the `find` vs `filter` distinction** — `find` never narrows the result set;
   export and paging always follow `filter`.
