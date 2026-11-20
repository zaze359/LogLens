// Mirrors src-tauri/src/model.rs. Field names are camelCase on the IPC boundary.

export interface LogEntry {
  index: number
  raw: string
  timestamp: string
  timeKey: string
  /** `YYYY-MM-DD HH:MM:SS.mmm`；没有年份的日志用 1970 作占位年份。无日期时为空。 */
  datetimeKey: string
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

/** 一条可复用的搜索历史（关键字 + 解释方式）。 */
export interface SearchHistoryEntry {
  keyword: string
  /** 'filter' | 'find' */
  mode: string
  isRegex: boolean
  caseSensitive: boolean
  /** epoch millis，新的在前 */
  ts: number
}

export interface QueryResult {
  total: number
  matched: number
  entries: LogEntry[]
  levelCounts: NameCount[]
  topTags: NameCount[]
  elapsedMs: number
}

/**
 * 命中日志在时间轴上的分布：把条目按 `timeKey` 均匀分桶。
 *
 * 轴的范围取自整份日志（`start` / `end`），桶里只数命中的条目，且统计时忽略
 * `timeFrom` / `timeTo` —— 选中一段区间不会把轴上的其他时间段抹掉。
 */
export interface TimeTimeline {
  /** 轴的起点标签：带年份时 `YYYY-MM-DD HH:MM:SS.mmm`，只有月日时 `MM-DD HH:MM:SS.mmm`。 */
  start: string
  /** 轴的终点标签，格式同 `start`。 */
  end: string
  /** 轴起点的绝对毫秒（1970-01-01 为原点），用于像素与时间的换算。 */
  startMs: number
  /** 轴终点的绝对毫秒。 */
  endMs: number
  /** 日志是否带日期；`false` 时轴上只有时刻。 */
  hasDate: boolean
  /** 日期是否带真实年份；`false` 时年份是占位值，展示与筛选都不包含它。 */
  hasYear: boolean
  /** 每个等宽桶内的命中条数；空数组表示没有可用的时间戳。 */
  bins: number[]
  /** 命中有时间戳的条目数（= `bins` 之和）。 */
  total: number
  /** 命中但读不出时间戳的条目数，它们不在轴上。 */
  untimed: number
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
  /** 展示用正文被上限截短；半截 JSON 不能格式化 */
  responseTruncated: boolean
  error: string
  /** har | jsonl | cdp */
  source: string
  /** 产生这条请求的客户端：设备抓包为串号，独立代理为来源 IP，文件导入为空 */
  device: string
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
  /** 非空表示本次体检自动清理了上一次运行残留的设备代理 */
  healedProxy: string
}

export interface CdpCaptureOptions {
  serial: string
  /** 留空则自动用体检找到的第一个 WebView 调试通道 */
  socket: string
  /** 0 = 用默认端口 */
  port: number
}

export interface ProxyCaptureOptions {
  serial: string
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
  /** 原生采集识别到的设备 IP；空表示没读到，流量会归入独立代理页 */
  deviceIp: string
}

export interface NetworkPollResult {
  id: string
  requests: NetworkRequest[]
  status: NetworkCaptureStatus
  added: number
}

/** 独立代理（本地 HTTP 代理）的状态，不绑定任何设备。 */
export interface ProxyServerStatus {
  running: boolean
  /** 客户端要指向的地址，如 172.21.32.185:18999 */
  endpoint: string
  lan: string
  port: number
  /** 已经连上来过的不同来源地址数量（独立代理页的分组） */
  sources: number
  /** 绑定在同一监听上的设备数量：抓包页与独立代理共用这一个代理 */
  devices: number
  error: string
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

/** 客户端自身的故障记录（与正在分析的日志无关），来自 Rust 的 error_log 模块。 */
export interface ErrorRecord {
  id: string
  /** epoch 毫秒（重复发生时更新为最后一次的时间） */
  ts: number
  /** panic | error | warning */
  level: string
  /** rust | frontend */
  source: string
  /** 命令名、模块名或前端处理器名 */
  scope: string
  message: string
  /** 调用栈，可能为空 */
  detail: string
  /** 短时间内相同故障折叠为一条 */
  count: number
}
