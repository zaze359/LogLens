# Hook Guidelines

> This is a Vue project, so there are no React hooks. This file documents the equivalent
> conventions: composables, watchers, timers and event subscriptions.

---

## Overview

- There is **no `composables/` directory and no custom `useXxx()` function** today.
- Shared stateful logic lives in `src/stores/session.ts` as exported functions that read and
  mutate the singleton `store`.
- Components keep only UI-local state in `ref` / `computed`.
- Side effects (timers, Tauri listeners) are owned by the module or component that creates
  them and must be released explicitly.

Do not introduce a composable for a single caller. Promote logic to the store when two
components need it; promote to `lib/` only when it is pure and stateless.

---

## Custom Composition Patterns

### Store functions as the shared-logic unit

```ts
// Any component can call this; the store owns sequencing, errors and timers.
import { updateSearch } from '../stores/session'

updateSearch('timeout')
```

The store module also exports `computed` values for shared derivation:
`activeTab`, `currentHitIndex`, `searching`, `visibleSections`, `totalRuleCount`.

### Local state stays local

`NetworkCaptureTab.vue` keeps `report`, `captures`, `statuses`, `requests`, `selected`,
`proxyInput` and its polling timer locally because nothing outside the tab reads them.
Only device selection and the active tool live in the global store.

---

## Data Fetching

- All request/response traffic goes through `src/lib/ipc.ts` (`ipc.queryLogs`, `ipc.searchHits`,
  `ipc.startCdpCapture`, ...). Each wrapper returns a typed `Promise`.
- The store applies responses; components normally call store actions
  (`refresh`, `loadMore`, `startDeviceStream`) rather than IPC directly.
- Long-running backend work is polled, not pushed, except import progress:
  - Import progress: `ipc.onOpenProgress(handler)` returns an `unlisten` function.
  - Live log capture: `pollDeviceStream` every `STREAM_POLL_MS = 800`.
  - Network capture: `pollNetworkCapture` on the same 800ms cadence.
- Handle `unknown` errors with `errorMessage(e)` from `lib/ipc.ts`; never `String(e)` inline.

---

## Watchers, Timers and Listeners

- Watchers must have a single, stated purpose. Existing examples:
  - watch `store.entries.length` to follow the live tail, gated by a `follow` ref.
  - watch `store.activeId` to restore the per-tab scroll offset after `nextTick()`.
  - watch `store.scrollTarget` to jump to a find hit; a `seq` field forces repeat jumps.
- Store `setTimeout` / `setInterval` handles in a module-level nullable variable, clear the
  previous handle before creating a new one, and clear them when the owner goes away.
- `setInterval` tick functions must be re-entrancy safe. `tickStreams` dispatches hidden tabs
  to `pollInBackground` and the active tab to `syncStream`; both use the same per-tab
  `polling` flag, so a slow poll cannot stack.
- Tauri event subscriptions return an unlisten function; store it and call it in
  `onBeforeUnmount` (`App.vue` drag & drop listener).
- An empty `catch` is allowed only for an optional enhancement or best-effort cleanup, and
  must say why in a comment.

---

## Naming Conventions

| Kind | Convention | Example |
|------|------------|---------|
| Shared action | verb + object | `loadFile`, `activateTab`, `scheduleFind`, `stopDeviceStream` |
| Debounced action | `scheduleXxx` | `scheduleRefresh`, `scheduleFind` |
| Derived value | noun / adjective | `activeTab`, `currentHitIndex`, `isFindMode` |
| Boolean | reads as a fact | `loading`, `follow`, `probing`, `busy` |
| Timer handle | `<purpose>Timer` | `refreshTimer`, `findTimer`, `curlTimer` |
| Event handler | `onXxx` for DOM, verb for actions | `onScroll`, `jumpToLatest` |

---

## Common Mistakes

1. **Creating `useLogStore()`** — the store is already a module singleton; a composable would
   add a second access path.
2. **Forgetting to clear a timer on unmount** — a stale polling loop keeps calling IPC after
   the pane is gone.
3. **Starting an interval without a re-entrancy guard** — responses arrive late and overwrite
   newer state.
4. **Subscribing to a Tauri event without keeping the unlisten function** — the listener
   leaks across hot reloads and window lifetimes.
5. **Doing fetch sequencing in a component** — sequence tokens (`requestSeq`, `findSeq`) live
   in the store and must protect every mutation of the shared view state.
