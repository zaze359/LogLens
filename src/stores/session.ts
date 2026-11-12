import { computed, reactive } from 'vue'
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog'
import { errorMessage, ipc, isTauri } from '../lib/ipc'
import { UNCATEGORIZED } from '../types'
import type {
  DeviceCaptureOptions,
  FileInfo,
  FilterSpec,
  LogEntry,
  QueryResult,
  Rule,
  RuleCategory,
  RuleGroup,
  RulesView,
  StreamStatus
} from '../types'

export const PAGE_SIZE = 400

/** How often a live capture is polled. Fast enough to feel live, cheap enough to ignore. */
const STREAM_POLL_MS = 800

/**
 * Tabs are capped to the backend's `MAX_OPEN_SESSIONS` (4). Going past it would make Rust
 * evict the oldest session behind the UI's back, leaving a tab whose every query fails.
 */
export const MAX_TABS = 4

/** A running `adb logcat` capture, mirrored in the UI. */
export interface DeviceStreamInfo {
  sessionId: string
  serial: string
  label: string
  running: boolean
  error: string
}

/**
 * One open log source. Each tab owns a backend session (same id) plus its own view state,
 * so switching tabs never mixes two logs' filters, scroll positions or live captures.
 *
 * `entries` / `result` are snapshots taken when the tab is switched **away**; a background
 * tab keeps receiving log lines in its backend session, not here — see `catchUp()`.
 */
export interface SessionTab {
  id: string
  /** Tab label: the file name, or `<device> · 实时 logcat`. */
  title: string
  /** `file` | `stream` */
  source: string
  info: FileInfo
  filter: FilterSpec
  result: QueryResult | null
  entries: LogEntry[]
  selected: LogEntry | null
  activeRuleId: string
  testCount: number | null
  scrollTop: number
  /** Non-null while this tab's `adb logcat` child is still attached. */
  stream: DeviceStreamInfo | null
  /** Guards the 800ms tick against stacking two background polls on one tab. */
  polling: boolean
}

export function emptyFilter(): FilterSpec {
  return {
    levels: [],
    tags: [],
    pids: [],
    packages: [],
    search: '',
    isRegex: false,
    caseSensitive: false,
    exclude: '',
    timeFrom: '',
    timeTo: '',
    group: null,
    groupLogic: 'AND',
    order: 'asc'
  }
}

export interface AppState {
  tauriReady: boolean
  /** Open tabs, in strip order. Empty means "no log open". */
  tabs: SessionTab[]
  /** Session id of the tab whose state the flat mirror fields below currently hold. */
  activeId: string
  // ---- mirror of the active tab, so every component can keep reading `store.xxx` ----
  fileInfo: FileInfo | null
  filter: FilterSpec
  result: QueryResult | null
  entries: LogEntry[]
  activeRuleId: string
  selected: LogEntry | null
  loading: boolean
  loadingMore: boolean
  error: string
  notice: string
  testCount: number | null
  /** Scroll offset of the active tab's log table. */
  scrollTop: number
  /** Non-null while the active tab is still receiving live lines. */
  deviceStream: DeviceStreamInfo | null
  // ---- global (shared by every tab) ----
  templates: Rule[]
  rules: Rule[]
  categories: RuleCategory[]
  ruleSearch: string
  /** Category id -> collapsed. Ignored while a search is active. */
  collapsed: Record<string, boolean>
  rulesPath: string
  /** Device picker dialog visibility. */
  devicePanel: boolean
  /** Last used capture options, so reopening the panel does not start from scratch. */
  lastCapture: DeviceCaptureOptions | null
}

export const store = reactive<AppState>({
  tauriReady: false,
  tabs: [],
  activeId: '',
  fileInfo: null,
  filter: emptyFilter(),
  result: null,
  entries: [],
  activeRuleId: '',
  selected: null,
  loading: false,
  loadingMore: false,
  error: '',
  notice: '',
  testCount: null,
  scrollTop: 0,
  deviceStream: null,
  templates: [],
  rules: [],
  categories: [],
  ruleSearch: '',
  collapsed: {},
  rulesPath: '',
  devicePanel: false,
  lastCapture: null
})

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

let noticeTimer: ReturnType<typeof setTimeout> | null = null
export function setNotice(text: string): void {
  store.notice = text
  if (noticeTimer) clearTimeout(noticeTimer)
  noticeTimer = setTimeout(() => {
    store.notice = ''
    noticeTimer = null
  }, 4500)
}

/** Every mutating rules IPC now returns the full bundle, so one call refreshes both lists. */
export function applyRuleBundle(view: RulesView): void {
  store.categories = view.categories ?? []
  store.rules = view.rules ?? []
}

export async function init(): Promise<void> {
  store.tauriReady = isTauri()
  if (!store.tauriReady) return
  try {
    const [templates, bundle] = await Promise.all([
      ipc.listBuiltinTemplates(),
      ipc.listRuleBundle().catch(() => ({ categories: [], rules: [] } as RulesView))
    ])
    store.templates = templates
    applyRuleBundle(bundle)
    void ipc.rulesLocation().then((p) => (store.rulesPath = p)).catch(() => {})
  } catch (e) {
    store.error = errorMessage(e)
  }
}

/* --------------------------------------------------------------- tabs ---- */

export const activeTab = computed<SessionTab | null>(
  () => store.tabs.find((tab) => tab.id === store.activeId) ?? null
)

/** Copy the live view state into the tab being left, so nothing is lost on a switch. */
function snapshotActive(): void {
  const tab = activeTab.value
  if (!tab) return
  // `filter` is deep-cloned: `updateFilter` mutates `store.filter` in place, and two tabs
  // must never end up sharing one object.
  tab.filter = clone(store.filter)
  tab.result = store.result
  tab.entries = store.entries
  tab.selected = store.selected
  tab.activeRuleId = store.activeRuleId
  tab.testCount = store.testCount
  tab.scrollTop = store.scrollTop
  tab.stream = store.deviceStream
}

function clearView(): void {
  store.activeId = ''
  store.fileInfo = null
  store.filter = emptyFilter()
  store.result = null
  store.entries = []
  store.selected = null
  store.activeRuleId = ''
  store.testCount = null
  store.scrollTop = 0
  store.deviceStream = null
  store.error = ''
  store.loading = false
  store.loadingMore = false
}

/** Make `id` the visible tab. Cheap: everything it needs was snapshotted on the way out. */
export function activateTab(id: string): void {
  if (id === store.activeId) return
  const tab = store.tabs.find((candidate) => candidate.id === id)
  if (!tab) return

  snapshotActive()
  // Responses still in flight belong to the tab being left; make them unlandable.
  requestSeq++
  paging = false

  store.fileInfo = tab.info
  store.filter = clone(tab.filter)
  store.result = tab.result
  store.entries = tab.entries
  store.selected = tab.selected
  store.activeRuleId = tab.activeRuleId
  store.testCount = tab.testCount
  store.scrollTop = tab.scrollTop
  store.deviceStream = tab.stream
  store.error = ''
  store.loading = false
  store.loadingMore = false
  // Set last: `LogTable` watches it to restore the scroll offset.
  store.activeId = id

  syncTimers()
  if (tab.stream) {
    // A background tab was not rendering its deltas, so pull them now — otherwise the
    // user comes back to a list that stops mid-burst.
    void syncStream().then(catchUp)
  }
}

export function closeTab(id: string): void {
  const index = store.tabs.findIndex((tab) => tab.id === id)
  if (index < 0) return
  const wasActive = id === store.activeId

  // Drop the tab first so the strip reacts instantly; the backend session (and that tab's
  // `adb logcat` child) is released in the background.
  store.tabs.splice(index, 1)
  void ipc.closeSession(id).catch(() => {})

  if (!wasActive) {
    syncTimers()
    return
  }
  const next = store.tabs[index] ?? store.tabs[index - 1] ?? null
  if (next) {
    // `activeId` still points at the closed tab, so `activateTab` restores instead of
    // early-returning.
    activateTab(next.id)
  } else {
    clearView()
    syncTimers()
  }
}

/** Open a session as a new tab and focus it. `stream` is set for live captures. */
async function openTab(info: FileInfo, stream: DeviceStreamInfo | null): Promise<void> {
  const tab: SessionTab = {
    id: info.sessionId,
    title: info.fileName,
    source: info.source,
    info,
    filter: emptyFilter(),
    result: null,
    entries: [],
    selected: null,
    activeRuleId: '',
    testCount: null,
    scrollTop: 0,
    stream,
    polling: false
  }
  store.tabs.push(tab)
  activateTab(tab.id)
  store.loading = true
  await refresh()
  store.loading = false
}

export async function loadFile(path: string): Promise<void> {
  if (!store.tauriReady) {
    setNotice('请在 Tauri 客户端中打开日志文件')
    return
  }
  // Same file already open? Focus that tab — a second session would show identical data
  // and burn one of the four slots.
  const existing = store.tabs.find((tab) => tab.source === 'file' && tab.info.path === path)
  if (existing) {
    activateTab(existing.id)
    setNotice(`该文件已在新标签中打开：${existing.title}`)
    return
  }
  if (store.tabs.length >= MAX_TABS) {
    setNotice(`最多同时打开 ${MAX_TABS} 个日志，先关掉一个标签再打开`)
    return
  }
  store.error = ''
  try {
    const info = await ipc.openLogFile(path)
    await openTab(info, null)
    setNotice(`已在新标签中载入 ${info.fileName}：${info.totalLines} 行 / ${info.entryCount} 条`)
  } catch (e) {
    store.error = errorMessage(e)
  }
}

export async function pickAndLoadFile(): Promise<void> {
  if (!store.tauriReady) {
    setNotice('请在 Tauri 客户端中使用文件选择')
    return
  }
  try {
    const picked = await openDialog({ multiple: false, directory: false, title: '选择日志文件' })
    if (typeof picked === 'string' && picked.length > 0) {
      await loadFile(picked)
    }
  } catch (e) {
    store.error = errorMessage(e)
  }
}

/** Opens the capture dialog. */
export function openDevicePanel(): void {
  if (!store.tauriReady) {
    setNotice('请在 Tauri 客户端中连接设备')
    return
  }
  store.devicePanel = true
}

export function closeDevicePanel(): void {
  store.devicePanel = false
}

/** Follow new device lines until the user stops or adb exits. Opens in its own tab. */
export async function startDeviceStream(options: DeviceCaptureOptions): Promise<boolean> {
  if (!store.tauriReady) {
    setNotice('请在 Tauri 客户端中连接设备')
    return false
  }
  // One device, one live capture. Two `adb logcat` children read the same ring buffer, and
  // "先清空缓冲区" on the second would wipe what the first is still collecting. Point at the
  // tab that already owns it instead of just refusing.
  const busy = store.tabs.find(
    (tab) => tab.stream && tab.info.source === 'stream' && tab.info.device === options.serial
  )
  if (busy) {
    activateTab(busy.id)
    setNotice(`设备 ${options.serial} 已在标签「${busy.title}」中实时抓取，先在那里停止再重新开始`)
    return false
  }
  if (store.tabs.length >= MAX_TABS) {
    setNotice(`最多同时打开 ${MAX_TABS} 个日志，先关掉一个标签再开始抓取`)
    return false
  }
  store.error = ''
  try {
    store.lastCapture = { ...options }
    const info = await ipc.startDeviceStream(options)
    const stream: DeviceStreamInfo = {
      sessionId: info.sessionId,
      serial: options.serial,
      label: info.fileName,
      running: true,
      error: ''
    }
    await openTab(info, stream)
    syncTimers()
    // `adb logcat` replays the existing buffer first, so a non-empty start is normal and
    // means "these lines already happened" — worth saying out loud once.
    const replayed = info.entryCount > 0 ? `（含设备缓冲区已有的 ${info.entryCount} 条）` : ''
    setNotice(`已在新标签开始实时抓取${replayed}，切到别的标签也会继续接收`)
    return true
  } catch (e) {
    store.error = errorMessage(e)
    return false
  }
}

let streamTimer: ReturnType<typeof setInterval> | null = null

/** The poller exists exactly while at least one tab is still receiving live lines. */
function syncTimers(): void {
  const needed = store.tabs.some((tab) => tab.stream)
  if (needed && !streamTimer) {
    streamTimer = setInterval(tickStreams, STREAM_POLL_MS)
  } else if (!needed && streamTimer) {
    clearInterval(streamTimer)
    streamTimer = null
  }
}

/** One tick covers every tab: the visible one repaints, the hidden ones only drain. */
function tickStreams(): void {
  for (const tab of [...store.tabs]) {
    if (!tab.stream) continue
    if (tab.id === store.activeId) void syncStream()
    else void pollInBackground(tab)
  }
}

/** Guards the "fetch the next page" offset against two concurrent callers (stream poll
 *  tick + scroll-triggered `loadMore`) both asking for the same offset. Shared by all tabs:
 *  switching tabs resets it, and a stale call only ever sees its own discarded response. */
let paging = false

/** Bumped whenever the visible session or filter changes, so slow responses cannot land on
 *  a tab they no longer belong to. */
let requestSeq = 0

/** Append newly matched entries. New lines always land at the end, so ascending order can
 *  keep the already-loaded page and just continue from `entries.length`. Descending order
 *  puts them on top, where a cheap re-query of page 0 is the correct move. */
async function appendNewMatches(): Promise<void> {
  const info = store.fileInfo
  if (!info || paging) return
  if (store.filter.order === 'desc') {
    await refresh()
    return
  }
  paging = true
  const loaded = store.entries.length
  const seq = requestSeq
  try {
    const result = await ipc.queryLogs(info.sessionId, store.filter, loaded, PAGE_SIZE)
    // A filter change (or another page load) landed while this query was in flight.
    if (seq !== requestSeq || store.entries.length !== loaded) return
    if (result.matched > loaded) {
      store.entries = store.entries.concat(result.entries)
    }
    store.result = result
  } finally {
    paging = false
  }
}

/** Rebuild a returning tab's list when lines arrived while it was hidden.
 *
 *  Bounded on purpose: a long absence should not turn one tab switch into dozens of
 *  queries. Whatever is left over is still reachable by scrolling (`loadMore`). */
async function catchUp(): Promise<void> {
  for (let round = 0; round < 12; round++) {
    const matched = store.result?.matched ?? 0
    const loaded = store.entries.length
    if (loaded >= matched) return
    await appendNewMatches()
    if (store.entries.length === loaded) return
  }
}

/** Mirror the counters (and, when they changed, the filter facets) onto the file info. */
function applyStreamStatus(info: FileInfo, status: StreamStatus): void {
  info.totalLines = status.totalLines
  info.entryCount = status.entryCount
  info.unparsedLines = status.unparsedLines
  if (status.facetsUpdated) {
    info.levels = status.levels
    info.tags = status.tags
    info.pids = status.pids
    info.packages = status.packages
  }
}

/** Pull the visible tab's live deltas into the table. */
async function syncStream(): Promise<void> {
  const info = store.fileInfo
  const device = store.deviceStream
  if (!info || !device) return
  const sessionId = info.sessionId
  try {
    const status: StreamStatus = await ipc.pollDeviceStream(sessionId)
    // `info` and `device` belong to this tab, so writing them is safe even if the user
    // switched away mid-call. The flat mirror fields are not — those get a guard.
    applyStreamStatus(info, status)
    if (store.fileInfo?.sessionId !== sessionId) return
    await appendNewMatches()

    if (!status.running) {
      device.running = false
      info.streaming = false
      const tab = store.tabs.find((candidate) => candidate.id === sessionId)
      if (tab) tab.stream = null
      store.deviceStream = null
      await ipc.stopDeviceStream(sessionId).catch(() => {})
      syncTimers()
      // Always name the tab: with several open, "抓取已结束" alone is ambiguous.
      setNotice(
        status.error
          ? `${device.label}：${status.error}`
          : `${device.label} 实时抓取已结束，共 ${status.entryCount} 条`
      )
    }
  } catch (e) {
    // The tab was closed, or the backend already cleaned the stream up. Both poll errors
    // mean the capture is over, so make sure no stale stream record blocks the device.
    const tab = store.tabs.find((candidate) => candidate.id === sessionId)
    if (tab) tab.stream = null
    void ipc.stopDeviceStream(sessionId).catch(() => {})
    if (store.fileInfo?.sessionId === sessionId) {
      store.deviceStream = null
      store.error = errorMessage(e)
    }
    syncTimers()
  }
}

/** Drain a hidden tab's stream so its backend buffer never grows unbounded, without
 *  touching the visible table. `activateTab` catches the table up on the way back in. */
async function pollInBackground(tab: SessionTab): Promise<void> {
  if (tab.polling || !tab.stream) return
  tab.polling = true
  try {
    const status = await ipc.pollDeviceStream(tab.id)
    applyStreamStatus(tab.info, status)
    if (!status.running) {
      tab.stream = null
      tab.info.streaming = false
      // Drop the backend entry as well: a device with a stale stream record would refuse
      // to start a new capture ("同一台设备同时只能有一个实时抓取") until the tab is closed.
      void ipc.stopDeviceStream(tab.id).catch(() => {})
      setNotice(
        status.error ? `${tab.title}：${status.error}` : `${tab.title} 实时抓取已结束`
      )
      syncTimers()
    }
  } catch (e) {
    tab.stream = null
    tab.info.streaming = false
    void ipc.stopDeviceStream(tab.id).catch(() => {})
    setNotice(`${tab.title}：${errorMessage(e)}`)
    syncTimers()
  } finally {
    tab.polling = false
  }
}

/** User-initiated stop for the visible tab: drain what is left, then kill the adb child. */
export async function stopDeviceStream(): Promise<void> {
  const info = store.fileInfo
  const tab = activeTab.value
  if (!info) {
    store.deviceStream = null
    return
  }
  try {
    const status = await ipc.pollDeviceStream(info.sessionId).catch(() => null)
    if (status) {
      applyStreamStatus(info, status)
      await appendNewMatches()
    }
    await ipc.stopDeviceStream(info.sessionId)
  } catch (e) {
    store.error = errorMessage(e)
  } finally {
    info.streaming = false
    store.deviceStream = null
    if (tab) tab.stream = null
    syncTimers()
    setNotice('已停止实时抓取，已收到的日志仍可继续筛选与导出')
  }
}

export async function refresh(): Promise<void> {
  if (!store.fileInfo) {
    store.result = null
    store.entries = []
    return
  }
  const seq = ++requestSeq
  store.loading = true
  try {
    const result = await ipc.queryLogs(store.fileInfo.sessionId, store.filter, 0, PAGE_SIZE)
    // A slower older request must not overwrite the result of a newer filter.
    if (seq !== requestSeq) return
    store.result = result
    store.entries = result.entries
    store.error = ''
  } catch (e) {
    if (seq !== requestSeq) return
    store.error = errorMessage(e)
  } finally {
    if (seq === requestSeq) store.loading = false
  }
}

export async function loadMore(): Promise<void> {
  if (!store.fileInfo || store.loadingMore || store.loading || paging) return
  const loaded = store.entries.length
  const matched = store.result?.matched ?? 0
  if (loaded >= matched) return
  const seq = requestSeq
  store.loadingMore = true
  try {
    const result = await ipc.queryLogs(store.fileInfo.sessionId, store.filter, loaded, PAGE_SIZE)
    // Discard pages that belong to an older filter or were already superseded.
    if (seq !== requestSeq || store.entries.length !== loaded) return
    store.entries = store.entries.concat(result.entries)
  } catch (e) {
    store.error = errorMessage(e)
  } finally {
    store.loadingMore = false
  }
}

let refreshTimer: ReturnType<typeof setTimeout> | null = null
export function scheduleRefresh(): void {
  if (refreshTimer) clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => {
    refreshTimer = null
    void refresh()
  }, 220)
}

export function updateFilter(partial: Partial<FilterSpec>, debounce = false): void {
  Object.assign(store.filter, partial)
  if (debounce) scheduleRefresh()
  else void refresh()
}

export function resetFilter(): void {
  store.filter = emptyFilter()
  store.activeRuleId = ''
  void refresh()
}

export function applyRule(rule: Rule): void {
  store.filter.group = clone(rule.group)
  store.activeRuleId = rule.id
  void refresh()
}

export function clearRule(): void {
  store.filter.group = null
  store.activeRuleId = ''
  void refresh()
}

export function newRule(categoryId: string = UNCATEGORIZED): Rule {
  const now = Date.now()
  return {
    id: `custom-${now}`,
    name: '新规则',
    description: '',
    color: '#38bdf8',
    builtin: false,
    categoryId,
    group: {
      logic: 'AND',
      conditions: [
        {
          id: `c-${now}`,
          field: 'message',
          op: 'contains',
          value: '',
          caseSensitive: false,
          negate: false
        }
      ]
    }
  }
}

export function ruleFromTemplate(template: Rule): Rule {
  const copy = clone(template)
  copy.id = `custom-${Date.now()}`
  copy.builtin = false
  copy.name = `${copy.name}（副本）`
  // Keep the template's category so the copy lands next to it in the sidebar.
  return copy
}

export function newCategory(): RuleCategory {
  return {
    id: '',
    name: '',
    description: '',
    color: '#34d399',
    builtin: false
  }
}

export async function saveRule(rule: Rule): Promise<void> {
  try {
    applyRuleBundle(await ipc.saveCustomRule(rule))
    setNotice(`已保存规则「${rule.name}」`)
  } catch (e) {
    store.error = errorMessage(e)
  }
}

export async function deleteRule(id: string): Promise<void> {
  try {
    applyRuleBundle(await ipc.deleteCustomRule(id))
    if (store.activeRuleId === id) clearRule()
    setNotice('规则已删除')
  } catch (e) {
    store.error = errorMessage(e)
  }
}

/** Returns the saved category id (generated by Rust for new ones), or null on failure. */
export async function saveCategory(category: RuleCategory): Promise<string | null> {
  const before = new Set(store.categories.map((c) => c.id))
  try {
    const view = await ipc.saveRuleCategory(category)
    applyRuleBundle(view)
    const created = view.categories.find((c) => !before.has(c.id) && !c.builtin)
    const id = category.id || created?.id || ''
    setNotice(`已保存分组「${category.name}」`)
    return id
  } catch (e) {
    store.error = errorMessage(e)
    return null
  }
}

export async function deleteCategory(id: string): Promise<void> {
  try {
    applyRuleBundle(await ipc.deleteRuleCategory(id))
    setNotice('分组已删除，组内规则已移到「未分组」')
  } catch (e) {
    store.error = errorMessage(e)
  }
}

export async function testRule(rule: Rule): Promise<void> {
  if (!store.fileInfo) {
    setNotice('请先打开日志文件再测试规则')
    return
  }
  const filter: FilterSpec = { ...emptyFilter(), group: clone(rule.group) }
  try {
    const result = await ipc.queryLogs(store.fileInfo.sessionId, filter, 0, 0)
    store.testCount = result.matched
    setNotice(`规则「${rule.name}」命中 ${result.matched} / ${result.total} 条`)
  } catch (e) {
    store.error = errorMessage(e)
  }
}

export async function exportFilteredFile(): Promise<void> {
  if (!store.fileInfo || !store.tauriReady) return
  try {
    const base = store.fileInfo.fileName.replace(/\.[^.]+$/, '')
    const dest = await saveDialog({
      title: '导出筛选结果',
      defaultPath: `${base}-filtered.log`,
      filters: [{ name: '日志文件', extensions: ['log', 'txt'] }]
    })
    if (!dest) return
    const count = await ipc.exportFiltered(store.fileInfo.sessionId, store.filter, dest)
    setNotice(`已导出 ${count} 条日志到 ${dest}`)
  } catch (e) {
    store.error = errorMessage(e)
  }
}

export async function exportRulesFile(): Promise<void> {
  if (!store.tauriReady) return
  try {
    const dest = await saveDialog({
      title: '导出自定义规则与分组',
      defaultPath: 'loglens-rules.json',
      filters: [{ name: 'JSON', extensions: ['json'] }]
    })
    if (!dest) return
    const count = await ipc.exportRulesToFilePath(dest)
    setNotice(`已导出 ${count} 条自定义规则（含分组）`)
  } catch (e) {
    store.error = errorMessage(e)
  }
}

export async function importRulesFile(): Promise<void> {
  if (!store.tauriReady) return
  try {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: '导入自定义规则与分组',
      filters: [{ name: 'JSON', extensions: ['json'] }]
    })
    if (typeof picked !== 'string' || !picked) return
    applyRuleBundle(await ipc.importRulesFromFilePath(picked))
    setNotice('规则与分组导入完成')
  } catch (e) {
    store.error = errorMessage(e)
  }
}

export function groupSummary(group: RuleGroup | null): string {
  if (!group || group.conditions.length === 0) return ''
  return `${group.conditions.length} 个条件 · ${group.logic}`
}

/* ---------------- grouping, search and collapse ---------------- */

export interface RuleSection {
  id: string
  name: string
  description: string
  color: string
  builtin: boolean
  /** Rules visible after applying the search query. */
  rules: Rule[]
  /** Rules in the section before searching. */
  total: number
}

export const uncategorisedCategory: RuleCategory = {
  id: UNCATEGORIZED,
  name: '未分组',
  description: '尚未归入任何分组的规则',
  color: '#64748b',
  builtin: false
}

function ruleHaystack(rule: Rule, category: RuleCategory): string {
  const parts: string[] = [
    category.name,
    category.description,
    rule.name,
    rule.description,
    rule.builtin ? '内置 模板 template builtin' : '自定义 规则 custom'
  ]
  for (const c of rule.group?.conditions ?? []) {
    parts.push(c.field, c.op, c.value)
  }
  return parts.join(' ').toLowerCase()
}

/** All whitespace-separated terms must match (AND). Searches names, descriptions,
 *  category names and condition values — not just the rule title. */
export function matchesQuery(rule: Rule, category: RuleCategory, query: string): boolean {
  const terms = query.toLowerCase().trim().split(/\s+/).filter(Boolean)
  if (terms.length === 0) return true
  const haystack = ruleHaystack(rule, category)
  return terms.every((term) => haystack.includes(term))
}

export const searching = computed<boolean>(() => store.ruleSearch.trim().length > 0)

export const totalRuleCount = computed<number>(
  () => store.templates.length + store.rules.length
)

export const visibleSections = computed<RuleSection[]>(() => {
  const all = [...store.templates, ...store.rules]
  const categories = [...store.categories, uncategorisedCategory]
  const query = store.ruleSearch.trim()

  const sections: RuleSection[] = categories.map((category) => {
    const inSection = all.filter((r) => (r.categoryId || UNCATEGORIZED) === category.id)
    return {
      id: category.id,
      name: category.name,
      description: category.description,
      color: category.color,
      builtin: category.builtin,
      total: inSection.length,
      rules: inSection.filter((r) => matchesQuery(r, category, query))
    }
  })

  // Built-in categories first, then user categories by name, uncategorised last.
  const rank = (s: RuleSection): number =>
    s.id === UNCATEGORIZED ? 2 : s.builtin ? 0 : 1
  return sections
    .filter((s) =>
      query ? s.rules.length > 0 : s.total > 0 || (!s.builtin && s.id !== UNCATEGORIZED)
    )
    .sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name, 'zh-Hans-CN'))
})

export const visibleRuleCount = computed<number>(() =>
  visibleSections.value.reduce((sum, s) => sum + s.rules.length, 0)
)

export function isCollapsed(id: string): boolean {
  // A search always expands matching sections, otherwise results look hidden.
  return !searching.value && store.collapsed[id] === true
}

export function toggleCategory(id: string): void {
  store.collapsed[id] = !store.collapsed[id]
}

export function expandAll(): void {
  store.collapsed = {}
}
