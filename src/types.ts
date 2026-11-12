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
  search: string
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
