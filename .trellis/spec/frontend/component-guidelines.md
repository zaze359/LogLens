# Component Guidelines

> How Vue components are written in this project.

---

## Overview

- Every component is a Single File Component with `<script setup lang="ts">`.
- No component defines a `<style>` block. Reusable classes come from `src/styles/main.css`;
  a number of layout/size values are set with inline `style` attributes (e.g.
  `CategoryEditor.vue`, `FilterBar.vue`).
- Props and emits are typed with the type-only macros.
- App state is imported from the store, not passed down through prop chains.
- Panes that own live work are kept mounted with `v-show`; dialogs use `v-if`.

---

## Component Structure

Standard order inside the file:

```vue
<script setup lang="ts">
import { computed, ref } from 'vue'
import { store } from '../stores/session'
import type { LogEntry } from '../types'

const props = defineProps<{ serial: string }>()
const emit = defineEmits<{ started: [] }>()

const busy = ref(false)
const ready = computed(() => props.serial.length > 0)
</script>

<template>
  <section class="device-page">...</section>
</template>
```

Rules:

1. Vue imports first, then local modules, then `import type`.
2. Constants and helper functions stay above the template, next to the logic that uses them.
3. Only export what the parent must call imperatively (`defineExpose`); everything else is
   internal. The single current example is `NetworkCaptureTab.vue:362`
   (`defineExpose({ shutdown, isCapturing: running })`).
4. Comments explain **why** a decision exists, not what the line does. Domain comments are
   Chinese; short technical notes may be English.

---

## Props Conventions

- Type-only declaration: `defineProps<{ serial: string; active: boolean }>()`.
- Required props only. No `withDefaults` is used today; if a default is needed, give the
  parent an explicit expression instead of hiding it in the child.
- Emits use the tuple/record syntax: `defineEmits<{ started: [] }>()`.
- Child components receive identity/context (`serial`, `active`), never a copy of the whole
  store. Example: `NetworkCaptureTab.vue` consumes `serial`, while the parent decides which
  device is active.
- Do not mutate props. Derive with `computed` and keep local editing state separate.

```ts
// src/components/device/NetworkCaptureTab.vue
const props = defineProps<{ serial: string; active: boolean }>()
const restoreTarget = computed(() => boundSerial.value || props.serial)
```

---

## Styling Patterns

- Global stylesheet, sectioned by feature with `/* ---------- area ---------- */`.
- Theme values are CSS variables declared at the top of `main.css`; use them instead of new
  hex colors (`var(--bg-1)`, `var(--text-dim)`, `var(--accent)`).
- Reuse shared classes: `btn`, `btn-sm`, `btn-primary`, `btn-ghost`, `input`, `select`,
  `check`, `chip`, `muted`, `mono`, `spacer`.
- Highlight hierarchy: `btn-primary` marks the single most common / expected action in the
  current context; alternatives and low-frequency paths stay plain `btn`. Do not use
  `btn-primary` for every entry point of a feature. Example: in device network capture,
  native proxy capture is primary and H5 capture is a plain button; a stop action uses
  `btn-danger-ghost`.
- Selection / active states all use the accent family: `var(--accent-soft)` background plus
  `var(--accent)` border or text. Shape may vary with the control (solid accent block for the
  device tool rail, tinted tab for session tabs, underline for detail sub-tabs, tinted row for
  table selection), but never introduce a separate blue. Green stays reserved for live /
  available status, yellow for find hits and warnings, red for destructive actions.
- Dynamic appearance comes from state classes, not inline style objects, except for values
  that are genuinely computed (column width / scroll transform in `LogTable.vue`).
- Row height and virtual scrolling constants are DOM/CSS-coupled:
  `ROW_H = 26` in `LogTable.vue` must match `.log-row` height in `main.css`. Changing one
  without the other silently misaligns virtualization.

---

## Pane Lifetime and Visibility

- **Keep stateful panes mounted.** `App.vue` renders one `LogPane` and one `DevicePage` per
  device, hidden with `v-show`. This keeps `adb logcat` polling, scroll position and local
  capture state alive when switching tools.
- **Use `v-if` for transient layers**: `DevicePickerDialog`, `ImportOverlay`, the error
  banner and the toast.
- Something that leaves state on the device (proxy, port forward) must register teardown:
  `DevicePage.vue` registers `shutdownOperations` through `registerDeviceCleanup`, and also
  calls it in `onBeforeUnmount`.

---

## Accessibility

The app is a dense desktop tool; there is no formal a11y test. Keep the existing baseline:

- Buttons and inputs are real `<button>` / `<input>` elements. The tab strip uses a `<div>`
  container with separate activation and close `<button>`s, so interactive elements are not
  nested.
- Icon-only buttons (`×`, `▣`) carry a `title`, plus an `aria-label` where the icon is the only
  content.
- Inputs generally have a visible label or placeholder describing the expected value.
- Custom focus styling exists for `.input` / `.select` in `main.css`; buttons and other
  controls keep the browser's default focus outline. Do not remove outlines without a
  replacement.

---

## Common Mistakes

1. **Building a second log table for device logs.** Device logs use the shared `LogPane`;
   only non-log tools live under `components/device/`.
2. **Add `<style scoped>` for one component.** Styles belong in `main.css`; scoped styles
   make the global class vocabulary drift.
3. **Call `invoke` inside a new component.** Add a wrapper to `lib/ipc.ts` instead. The one
   exception is `NetworkCaptureTab.vue`, which also imports `ipc` for capture-specific calls;
   do not add a third pattern.
4. **Unmount a pane that owns a timer or capture.** Use `v-show` and clean up the timer.
5. **Hardcode a color or a pixel width.** Use variables and the existing constants.
6. **Copy-paste a formatter** such as `formatSize` / `statusText`. If a second consumer
   appears, move it to `lib/`.
