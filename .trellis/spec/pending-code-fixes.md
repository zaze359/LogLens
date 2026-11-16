# Pending Code Fixes

> Fix record for spec-vs-code gaps. All four items below were fixed on **2026-11-16**; the
> entries are kept as a record and as the template for future gaps.

Status: all four items fixed.

| # | Area | Location | Status |
|---|------|----------|--------|
| 1 | Tab close control was a `<span role="button">` nested in a `<button>` | `src/components/SessionTabs.vue`, `src/styles/main.css` | Fixed |
| 2 | `RuleSidebar` confirm timer was not cleared on unmount | `src/components/RuleSidebar.vue` | Fixed |
| 3 | Icon-only close buttons had no accessible name | `CategoryEditor.vue`, `ConditionEditor.vue`, `NetworkCaptureTab.vue` | Fixed |
| 4 | Visible-tab stream poll had no re-entrancy guard | `src/stores/session.ts` (`syncStream`) | Fixed |

Not planned (documented as current behavior instead):

- Inline `style` attributes in components. They are widespread (layout/size values) and a
  mass refactor is not worth the churn; the spec describes them instead of banning them.
- `unwrap`/`expect` on static regexes and proven invariants. The spec bans them only for user
  input, file IO and locks in command paths.

---

## 1. Tab close control breaks button semantics — Fixed

**Was**: `src/components/SessionTabs.vue` rendered each tab as a `<button>` containing a
`<span class="tab-x" role="button">×</span>`, producing nested interactive content.

**Fix**: The tab is now a `<div class="tab">` containing two real `<button>`s:
`button.tab-main` (activation, with the title) and `button.tab-x` (close, with `title` and
`aria-label`). `src/styles/main.css` adds a `.tab-main` reset and resets `.tab-x` from span to
button.

**Result**: No nested interactive elements; close is keyboard-activatable and separately
focusable. Tab switching and closing keep the existing look.

---

## 2. `RuleSidebar` confirm timer outlives the component — Fixed

**Was**: `confirmTimer` was declared and armed with a 4s `setTimeout`, but the component had no
`onBeforeUnmount`.

**Fix**: `src/components/RuleSidebar.vue` imports `onBeforeUnmount` and clears `confirmTimer`
(and nulls it) when the sidebar unmounts.

**Result**: No callback runs against an unmounted component; matches the cleanup pattern used
by `NetworkCaptureTab` and `DeviceCaptureBar`.

---

## 3. Icon-only close buttons lack an accessible name — Fixed

**Was**: Three close buttons rendered only `×` with no `title` / `aria-label`:
`CategoryEditor.vue`, `ConditionEditor.vue`, `NetworkCaptureTab.vue`.

**Fix**: Each now has `title` plus `aria-label` — `关闭` for the two editors and
`关闭请求详情` for the network request detail.

**Result**: Hover and assistive technology report a meaningful action for each icon button.

---

## 4. Visible-tab stream poll can stack — Fixed

**Was**: `pollInBackground` guarded hidden tabs with `tab.polling`, but `syncStream` (visible
tab) had no guard and could overlap when a poll exceeded the 800ms tick.

**Fix**: `src/stores/session.ts` resolves the tab from the current `sessionId`, skips the poll
when `tab.polling` is set, sets it before the request, and resets it in `finally`. The two
inner `store.tabs.find(...)` lookups were removed in favor of that single tab reference.

**Result**: Visible and hidden tabs share the same per-tab re-entrancy rule; a slow poll no
longer stacks queries or appends.

---

## Verification

- `npm run build` (icon generation + `vue-tsc --noEmit` + `vite build`) passed on 2026-11-16.
- Manual smoke check still recommended: tab switch/close by mouse and keyboard, rule delete
  confirmation, dialog close buttons, and a high-volume live capture for duplicate-row checks.
- No Rust change was needed for these items.
