# Type Safety

> The IPC boundary is the only place where two languages meet. Types there are a contract,
> not a convenience.

---

## Overview

- TypeScript `strict: true`, plus `noUnusedLocals` and `noUnusedParameters`.
- `src/types.ts` is the **frontend half** of the wire contract. The Rust half is mostly
  `src-tauri/src/model.rs`, plus domain payloads defined in `adb.rs`, `network.rs`,
  `netcapture.rs`, `netprobe.rs` and `config_store.rs`. Change the matching Rust type and its
  TS mirror together.
- Rust serializes with `#[serde(rename_all = "camelCase", default)]`; TypeScript uses
  camelCase field names.
- There is no runtime validation library (no Zod/Yup). Trust the Rust side, and use
  `unknown` + a local cast only at genuinely external boundaries (Tauri event payloads,
  user-supplied files).

---

## Type Organization

| Location | Contents |
|----------|----------|
| `src/types.ts` | IPC payload mirrors (core `model.rs` types plus domain payloads), plus UI constants derived from backend semantics |
| `src/stores/session.ts` | UI-only types: `DeviceStreamInfo`, `SessionTab`, `ImportTask`, `AppState`, `DeviceOpId` |
| `src/lib/ipc.ts` | Command signatures; imports payload types from `types.ts` |
| Component files | Local row/segment/dialog types only (e.g. `Segment` in `LogTable.vue`) |

Do not create a second definition of an IPC payload inside a component.

---

## Contract Rules

1. **One field, two files.** Adding a field to `LogEntry` means editing `model.rs` and
   `types.ts` in the same change.
2. **Defaults over optionals.** Every Rust wire struct derives `Default` and uses
   `#[serde(default)]`. Missing fields deserialize to empty strings / empty vectors, so TS
   interfaces are non-optional. Only genuinely optional structures use `| null`
   (`FilterSpec.group`).
3. **Strings carry enums.** Wire enums are strings (`source: 'file' | 'stream'`,
   `searchMode: 'filter' | 'find'`, `op`, `field`, `order`). TS documents the allowed values
   in comments; it cannot enforce them without a runtime union type.
4. **Keep UI enumerations aligned with Rust.** `FIELDS`, `OPERATORS`, `LEVELS`,
   `LEVEL_LABELS` in `types.ts` are the UI's single source; `rules.rs` must accept every
   `op` / `field` they expose.
5. **`plain()` before `invoke`.** Vue reactive proxies cannot cross the Tauri boundary.
   `ipc.ts` JSON-round-trips filter/rule/options payloads. Any new command taking a reactive
   object must do the same.

```ts
// src/lib/ipc.ts
function plain<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

startProxyCapture: (options: ProxyCaptureOptions) =>
  invoke<NetworkSessionInfo>('start_proxy_capture', { options: plain(options) })
```

---

## Handling `unknown`

- `invoke` rejections are `unknown`. Normalize with `errorMessage(e)`:

```ts
export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message
  try { return JSON.stringify(e) } catch { return String(e) }
}
```

- Event payloads are `unknown` until narrowed. The one current cast is the drag & drop
  payload in `App.vue` because Tauri's event type is not exported there:

```ts
const payload = (event as { payload?: { type?: string; paths?: string[] } } | null)?.payload
```

- If a new external JSON format is parsed in the frontend, add a type guard
  `isXxx(value): value is Xxx` next to the type and parse through it. Do not scatter
  `as { field?: T }` casts (see the cross-layer guide).

---

## Common Patterns

- `computed<T>` for derived values; let TypeScript infer the element type where obvious.
- `Record<string, T>` for maps (`deviceOps`, `statuses`, `collapsed`).
- `InstanceType<typeof ChildComponent>` for template refs:
  `const networkTab = ref<InstanceType<typeof NetworkCaptureTab> | null>(null)`.
- `ReturnType<typeof setTimeout>` for timer handles (works in both DOM and Node typings).
- `as const` for literal option tables (`FIELDS`, `OPERATORS`).

---

## Forbidden Patterns

| Pattern | Rule |
|---------|------|
| `any` | Never. Use `unknown` plus narrowing, or a real interface |
| `as SomeInterface` on an IPC response | Unnecessary: `invoke<T>` is already typed |
| Optional (`?`) for backend fields | Rust sends defaults; mark them required |
| Duplicating a payload type locally | Import from `types.ts` |
| `// @ts-ignore` / `@ts-expect-error` | Requires a written reason and a narrow scope; prefer fixing the type |
| Numeric enums for wire values | The contract uses strings on both sides |

---

## Adding a Wire Field — Checklist

- [ ] Add the field to the Rust struct with `#[serde(rename_all = "camelCase", default)]`
- [ ] Add the same field to the TS interface in `src/types.ts`
- [ ] Populate it in every Rust construction site (`..Default::default()` hides omissions)
- [ ] Check JSON parsers (`config_store.rs`, `network.rs`) tolerate old files without it
- [ ] If it is user-editable, update the rule/filter editor option tables
- [ ] Run `cargo test --lib` and `npm run build`
