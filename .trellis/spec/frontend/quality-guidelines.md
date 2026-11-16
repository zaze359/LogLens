# Quality Guidelines

> What "done" means for a frontend change in this repo.

---

## Overview

There is no ESLint or Prettier configuration, and no frontend unit-test runner. Quality is
enforced by:

1. TypeScript strict mode + `vue-tsc --noEmit`
2. A successful production build
3. Manual smoke checks in `npm run dev` / `npm run tauri dev`

Do not add a formatter/linter setup as part of an unrelated change.

---

## Verification Commands

```bash
npm run build        # gen-icon + vue-tsc --noEmit + vite build
npm run dev          # browser preview; native features are disabled
npm run tauri dev    # full desktop app (file IO, adb, rules persistence)
npm run check        # scripts/setup.sh --check-only: toolchain/env report
```

Browser preview is expected to show the "请在 Tauri 客户端中运行" banner. Native features
must be verified in `npm run tauri dev`, not in a plain browser.

---

## Forbidden Patterns

| Pattern | Why |
|---------|-----|
| `any`, `<any>`, `as any` | Defeats the strict type check the project relies on |
| Calling `invoke('command')` in a component | Command names and arg shapes must stay centralized in `lib/ipc.ts` |
| Adding Pinia / Vuex / a router | The app is a single-window, single-store tool; the extra machinery is not warranted |
| `<style scoped>` or per-component CSS files | All styling lives in `styles/main.css` |
| `fetch` / `XMLHttpRequest` to the device or network | Device access belongs in Rust; the webview has no network privileges |
| Re-implementing logcat parsing, filtering or rule matching in TS | Rust is the source of truth; the UI only displays results |
| `localStorage` for app state | State is per-process; persisted user data is Rust's `rules.json` |
| Removing the `plain()` call before `invoke` | Vue reactive proxies are not structured-cloneable across IPC |
| `console.log` left in a hot path | Polling every 800ms makes it a performance and noise problem |
| A new dependency for a one-off helper | Prefer a small local function; runtime deps are Vue + Tauri APIs only |

---

## Required Patterns

- `<script setup lang="ts">` and typed `defineProps` / `defineEmits`.
- All native calls through `lib/ipc.ts`; all errors through `errorMessage(e)`.
- Store mutations through exported functions, with a race guard when async.
- Timers and listeners are cleared on unmount: `NetworkCaptureTab` clears its poll interval
  and curl timeout, `DeviceCaptureBar` clears its interval, and `RuleSidebar` clears its
  delete-confirm timer.
- Keep the existing formatting style: 2-space indent, single quotes, no semicolons,
  multi-line object literals use trailing commas where already present.
- Comments explain non-obvious decisions ("why"), especially for timing, ordering and
  cross-layer coupling. Match the surrounding language (Chinese for domain rationale).

---

## Testing Requirements

- Run `npm run build` before handing off. Fix all `vue-tsc` errors and unused-local errors
  (`noUnusedLocals` / `noUnusedParameters` are on).
- For UI behavior, verify in the desktop app:
  - open a log file, filter, page, switch tabs, reopen a tab (scroll/filter preserved)
  - live capture starts, appends, stops; switching tools does not interrupt it
  - import card appears, progresses and can be cancelled
  - rules save/delete persist across restart
- There is no frontend test harness. If pure logic is added (e.g. a formatter), keep it in
  `lib/` and make it trivially inspectable; do not introduce a test framework casually.

---

## Code Review Checklist

- [ ] Wire type change is mirrored between its Rust definition (`model.rs` or the owning domain module) and `src/types.ts`
- [ ] New command has an `ipc.ts` wrapper; no raw `invoke` in components
- [ ] Async store mutation is protected by `requestSeq` / `findSeq` / `polling`
- [ ] New state has one owner; per-tab state is snapshotted on switch
- [ ] Timers/listeners are cleared on unmount
- [ ] Pane that owns live work is `v-show`, not `v-if`
- [ ] New style uses variables and existing classes; `ROW_H` still matches CSS
- [ ] User-facing text is Chinese and actionable
- [ ] `npm run build` passes
