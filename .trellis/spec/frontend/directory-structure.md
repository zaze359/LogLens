# Directory Structure

> How `src/` is organized. The layout is flat on purpose: one app, no routing, no feature
> packages.

---

## Directory Layout

```
src/
├── main.ts                  # createApp(App).mount('#app') + styles import
├── App.vue                  # App shell: top bar, tabs, banner, workspace switching
├── types.ts                 # Wire types mirrored from model.rs + domain modules + UI constants
├── vite-env.d.ts            # Vite client type reference
├── lib/
│   ├── ipc.ts               # The only Tauri invoke/listen surface + errorMessage()
│   └── curl.ts              # Pure formatting helper: NetworkRequest -> cURL command
├── stores/
│   └── session.ts           # The single global store and every mutation function
├── styles/
│   └── main.css             # All styles: CSS variables, layout, controls, component classes
└── components/
    ├── TopBar.vue           # File actions, compare/export entry points
    ├── SessionTabs.vue      # Log/device workspace tab strip
    ├── LogPane.vue          # Log workspace layout: filter + table + detail + stats
    ├── FilterBar.vue        # Quick filters, search mode, rule application
    ├── LogTable.vue         # Virtualized log table, live tail, find highlighting
    ├── LogDetail.vue        # Selected entry detail
    ├── StatsPanel.vue       # Level distribution / top tags
    ├── RuleSidebar.vue      # Rule categories, search, create/delete
    ├── CategoryEditor.vue   # Category dialog
    ├── ConditionEditor.vue  # Rule condition editor
    ├── ImportOverlay.vue    # File import progress/cancel card
    └── device/              # Everything specific to a connected Android device
        ├── DevicePage.vue        # Non-log tool host; owns device-side cleanup
        ├── DeviceToolRail.vue    # Left icon rail: log / network
        ├── DeviceCaptureBar.vue  # Live capture status + stop button
        ├── DevicePickerDialog.vue# Connect/switch device
        ├── LogCaptureTab.vue     # "This device has no log yet" capture entry
        ├── NetworkCaptureTab.vue # Probe, CDP/proxy capture, request table, curl
        └── operations.ts         # DEVICE_OPERATIONS: the tool list rendered by rail/page
```

---

## Module Organization

### Layers

1. `types.ts` — wire contract only. No behavior.
2. `lib/` — stateless adapters/helpers. `ipc.ts` is the only place that knows command names.
3. `stores/session.ts` — application state and orchestration. It may import `ipc` and `types`.
4. `components/` — rendering and local interaction state. They import the store directly;
   there is no props drilling of app state.

Dependency direction is one-way: `components -> stores -> lib -> types`. A component must
not be imported by the store.

### Where new code goes

| Change | Location |
|--------|----------|
| New Rust command consumed by the UI | Add wrapper to `src/lib/ipc.ts`, type to `src/types.ts` |
| New shared app state or workflow | `src/stores/session.ts` |
| New screen region or dialog | `src/components/`, or `src/components/device/` if device-only |
| New device tool | `operations.ts` entry + `DevicePage.vue` body + cleanup in `shutdownOperations()` |
| New pure formatter (cURL, size, label) | `src/lib/` when reused; keep local when single-use |
| New shared style | `src/styles/main.css` under the existing section comments |

### Device vs. file workspaces

Device-only UI belongs in `components/device/`. The log pane is deliberately shared:
`App.vue` renders **one** `LogPane` for both imported files and live device logs. Do not
clone the log table into the device folder.

---

## Naming Conventions

| Kind | Rule | Example |
|------|------|---------|
| Component file | PascalCase `.vue`, one component per file | `DevicePickerDialog.vue` |
| Module file | camelCase `.ts` | `session.ts`, `operations.ts` |
| Exported type/interface | PascalCase | `SessionTab`, `ImportTask` |
| Store fields | camelCase nouns; booleans read as state (`loading`, `deviceView`) | `activeSerial`, `searchHits` |
| Store functions | verb-first (`loadFile`, `activateTab`, `scheduleRefresh`) | `closeDeviceWorkspace` |
| Constants | UPPER_SNAKE_CASE | `PAGE_SIZE`, `MAX_TABS`, `STREAM_POLL_MS` |
| CSS classes | kebab-case, often prefixed by area (`net-`, `device-page-`) | `.table-head`, `.lv-error` |
| No barrel files | Import the concrete path | `import { store } from '../stores/session'` |

---

## Examples

- Shell/workspace switching: `src/App.vue`
- Virtualized rendering + live tail: `src/components/LogTable.vue`
- Store orchestration with races and timers: `src/stores/session.ts`
- Device-owned cleanup: `src/components/device/DevicePage.vue`
- Long-lived pane kept mounted: `App.vue` (`v-show` on `LogPane` / `DevicePage`)
