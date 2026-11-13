// Mirrors src-tauri/src/model.rs. Field names are camelCase on the IPC boundary.

export interface LogEntry {
  index: number
  raw: string
  timestamp: string
  timeKey: string
  level: string
  tag: string
  pid: number
  tid: number
  package: string
  /** AccessibilityEvent.className for foreground-app events; empty for ordinary lines. */
  component: string
  message: string
  continuation: number
  parseOk: boolean
}

export interface Condition {
  id: string
  field: string
  op: string
  value: string
  caseSensitive: boolean
  negate: boolean
}

export interface RuleGroup {
  logic: string
  conditions: Condition[]
}

/** A named bucket of rules. Pure classification — never bound to an application. */
export interface RuleCategory {
  id: string
  name: string
  description: string
  color: string
  builtin: boolean
}

/** Empty id means "uncategorised" and is always rendered as the last section. */
export const UNCATEGORIZED = ''

export interface Rule {
  id: string
  name: string
  description: string
  color: string
  builtin: boolean
  categoryId: string
  group: RuleGroup
}

/** One round trip gives the sidebar everything it needs except built-in templates. */
export interface RulesView {
  categories: RuleCategory[]
  rules: Rule[]
}

export interface FilterSpec {
  levels: string[]
  tags: string[]
  pids: string[]
  /** 应用包名（子进程归并到所属包）。多选取并集。 */
  packages: string[]
  search: string
  /**
   * `filter`（默认）= 只显示匹配的行；`find` = 只高亮定位，不改变结果集。
   * 两种模式共用同一个关键字，切换模式决定它是「筛选」还是「查找」。
   */
  searchMode: string
  isRegex: boolean
  caseSensitive: boolean
  exclude: string
  timeFrom: string
  timeTo: string
  group: RuleGroup | null
  groupLogic: string
  order: string
}

export interface NameCount {
  name: string
  count: number
}

export interface FileInfo {
  sessionId: string
  path: string
  fileName: string
  sizeBytes: number
  totalLines: number
  entryCount: number
  unparsedLines: number
  levels: string[]
  tags: string[]
  pids: string[]
  packages: string[]
  /** `file`（导入日志文件）| `stream`（设备实时抓取） */
  source: string
  /** 设备序列号；文件来源为空 */
  device: string
  /** 实时抓取是否仍在追加 */
  streaming: boolean
}

/** `adb version` 探测结果。未安装 adb 时 available 为 false 且 error 带安装指引。 */
export interface AdbStatus {
  available: boolean
  path: string
  version: string
  error: string
}

export interface AdbDevice {
  serial: string
  /** `device`（可抓取）| `unauthorized` | `offline` | `no permissions` | ... */
  state: string
  model: string
  product: string
  device: string
  transportId: string
}

/**
 * 设备抓取参数。只有一种模式：持续跟随的实时流。
 * `adb logcat` 启动时本身会先回放设备现有缓冲区，所以「已经发生的问题」也看得到。
 */
export interface DeviceCaptureOptions {
  serial: string
  /** main / system / crash / events / all；空数组用 adb 默认缓冲区 */
  buffers: string[]
  /** 先执行 `adb logcat -c` 清空设备缓冲区；默认 false，清空不可恢复 */
  clearBefore: boolean
}

export interface StreamStatus {
  sessionId: string
  running: boolean
  entryCount: number
  totalLines: number
  unparsedLines: number
  error: string
  /** 本次轮询有新条目时，下面的候选列表才是最新的 */
  facetsUpdated: boolean
  levels: string[]
  tags: string[]
  pids: string[]
  packages: string[]
}

/** One `log-open-progress` event from `open_log_file`. */
export interface OpenProgress {
  /** Echo of the id the caller passed to `openLogFile`, so parallel imports can't mix. */
  requestId: string
  bytesRead: number
  totalBytes: number
  /** Entries parsed so far — one entry may span a whole stack trace. */
  entries: number
  /** `parsing` while reading, `indexing` while building facets afterwards. */
  phase: string
}

/** 查找（`find` 模式）的命中位置：只报告「在哪儿」，不参与过滤。 */
export interface SearchHits {
  /** 命中总数（不受 limit 影响）。 */
  total: number
  /** 按显示顺序排列的条目 index，最多 limit 个。 */
  indices: number[]
  /** index 列表被 limit 截断（总数仍然准确）。 */
  truncated: boolean
}

export interface QueryResult {
  total: number
  matched: number
  entries: LogEntry[]
  levelCounts: NameCount[]
  topTags: NameCount[]
  elapsedMs: number
}

export const FIELDS = [
  { value: 'message', label: '消息内容' },
  { value: 'tag', label: 'Tag' },
  { value: 'level', label: '级别' },
  { value: 'package', label: '包名' },
  { value: 'component', label: '组件类名' },
  { value: 'pid', label: 'PID' },
  { value: 'tid', label: 'TID' },
  { value: 'timestamp', label: '时间' },
  { value: 'timeKey', label: '归一化时间' },
  { value: 'raw', label: '原始行' }
] as const

export const OPERATORS = [
  { value: 'contains', label: '包含' },
  { value: 'notContains', label: '不包含' },
  { value: 'equals', label: '等于' },
  { value: 'notEquals', label: '不等于' },
  { value: 'startsWith', label: '开头是' },
  { value: 'endsWith', label: '结尾是' },
  { value: 'regex', label: '正则匹配' },
  { value: 'in', label: '属于(逗号分隔)' },
  { value: 'gte', label: '大于等于' },
  { value: 'lte', label: '小于等于' }
] as const

// --- 设备网络抓包 ---

export interface HttpHeader {
  name: string
  value: string
}

export interface NetworkRequest {
  index: number
  startedAt: string
  /** HH:MM:SS.mmm，与 logcat 时间轴对齐用的键 */
  timeKey: string
  method: string
  url: string
  host: string
  path: string
  scheme: string
  /** 0 = 未收到响应 */
  status: number
  statusText: string
  mimeType: string
  durationMs: number
  requestSize: number
  responseSize: number
  requestHeaders: HttpHeader[]
  responseHeaders: HttpHeader[]
  requestBody: string
  responseBody: string
  error: string
  /** har | jsonl | cdp */
  source: string
  redacted: boolean
}

export interface NetworkCaptureStatus {
  sessionId: string
  running: boolean
  entryCount: number
  events: number
  target: string
  error: string
}

export interface PackageProbe {
  package: string
  debuggable: boolean
  targetSdk: number
}

export interface DevtoolsProbe {
  pid: number
  socket: string
  package: string
  browser: string
  /** 0 表示通道在、但当前没有打开的 H5 页面 */
  pages: number
}

export interface DeviceEvidence {
  serial: string
  androidRelease: string
  sdk: number
  buildType: string
  debuggableBuild: boolean
  rootShell: boolean
  adbRootLikely: boolean
  proxy: string
  packages: PackageProbe[]
  devtools: DevtoolsProbe[]
}

export interface CaptureRoute {
  /** cdp | mitm-root | mitm-debug | proxy-metadata */
  id: string
  label: string
  httpsPlaintext: boolean
  coverage: string
  available: boolean
  reason: string
  steps: string[]
}

export interface ProbeReport {
  evidence: DeviceEvidence
  routes: CaptureRoute[]
  recommended: string
  summary: string
}

export interface CdpCaptureOptions {
  serial: string
  /** 留空则自动用体检找到的第一个 WebView 调试通道 */
  socket: string
  /** 0 = 用默认端口 */
  port: number
}

export interface NetworkSessionInfo {
  id: string
  label: string
  source: string
  device: string
  streaming: boolean
  count: number
}

export interface NetworkPollResult {
  id: string
  requests: NetworkRequest[]
  status: NetworkCaptureStatus
  added: number
}

export interface ProxyState {
  serial: string
  proxy: string
  original: string
  active: boolean
}

export const LEVELS = ['V', 'D', 'I', 'W', 'E', 'F', 'A', 'S', '?']

export const LEVEL_LABELS: Record<string, string> = {
  V: 'Verbose',
  D: 'Debug',
  I: 'Info',
  W: 'Warn',
  E: 'Error',
  F: 'Fatal',
  A: 'Assert',
  S: 'Silent',
  '?': 'Unparsed'
}
