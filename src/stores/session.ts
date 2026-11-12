import { computed, reactive } from 'vue'
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog'
import { errorMessage, ipc, isTauri } from '../lib/ipc'
import { UNCATEGORIZED } from '../types'
import type {
  FileInfo,
  FilterSpec,
  LogEntry,
  QueryResult,
  Rule,
  RuleCategory,
  RuleGroup,
  RulesView
} from '../types'

export const PAGE_SIZE = 400

export function emptyFilter(): FilterSpec {
  return {
    levels: [],
    tags: [],
    pids: [],
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
  fileInfo: FileInfo | null
  filter: FilterSpec
  result: QueryResult | null
  entries: LogEntry[]
  templates: Rule[]
  rules: Rule[]
  categories: RuleCategory[]
  ruleSearch: string
  /** Category id -> collapsed. Ignored while a search is active. */
  collapsed: Record<string, boolean>
  rulesPath: string
  activeRuleId: string
  selected: LogEntry | null
  loading: boolean
  loadingMore: boolean
  error: string
  notice: string
  testCount: number | null
}

export const store = reactive<AppState>({
  tauriReady: false,
  fileInfo: null,
  filter: emptyFilter(),
  result: null,
  entries: [],
  templates: [],
  rules: [],
  categories: [],
  ruleSearch: '',
  collapsed: {},
  rulesPath: '',
  activeRuleId: '',
  selected: null,
  loading: false,
  loadingMore: false,
  error: '',
  notice: '',
  testCount: null
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

export async function loadFile(path: string): Promise<void> {
  if (!store.tauriReady) {
    setNotice('请在 Tauri 客户端中打开日志文件')
    return
  }
  store.loading = true
  store.error = ''
  try {
    if (store.fileInfo) {
      await ipc.closeSession(store.fileInfo.sessionId).catch(() => {})
    }
    const info = await ipc.openLogFile(path)
    store.fileInfo = info
    store.entries = []
    store.selected = null
    store.activeRuleId = ''
    store.filter = emptyFilter()
    await refresh()
    setNotice(`已载入 ${info.fileName}：${info.totalLines} 行 / ${info.entryCount} 条`)
  } catch (e) {
    store.error = errorMessage(e)
  } finally {
    store.loading = false
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

let requestSeq = 0

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
  if (!store.fileInfo || store.loadingMore || store.loading) return
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
