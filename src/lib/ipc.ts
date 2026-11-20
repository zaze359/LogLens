import { invoke as rawInvoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { reportClientError } from './error-report'
import type {
  AdbDevice,
  AdbStatus,
  DeviceCaptureOptions,
  ErrorRecord,
  FileInfo,
  FilterSpec,
  OpenProgress,
  QueryResult,
  Rule,
  RuleCategory,
  RulesView,
  SearchHistoryEntry,
  SearchHits,
  StreamStatus,
  CdpCaptureOptions,
  NetworkPollResult,
  NetworkRequest,
  NetworkSessionInfo,
  ProbeReport,
  ProxyCaptureOptions,
  ProxyServerStatus,
  ProxyState,
  TimeTimeline
} from '../types'

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/** Vue reactive proxies are flattened before crossing the IPC boundary. */
function plain<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

/**
 * Every command failure is a client-side anomaly worth a diagnostic record — the UI
 * banner is transient, but "为什么刚才点了没反应" needs a durable trail.
 *
 * Repeated failures fold into one record in Rust, so the 800ms poll commands below are
 * excluded: they fail routinely during teardown (session already closed) and would
 * otherwise dominate the log. Their real errors already surface in StreamStatus /
 * NetworkCaptureStatus and the UI acts on them.
 */
const SILENT_COMMANDS = new Set([
  'poll_device_stream',
  'poll_network_capture',
  'report_client_error',
  // Recording a failure of the diagnostics channel into that same channel cannot work
  // and would recurse: report -> failed list -> report -> ...
  'list_error_log',
  'clear_error_log',
  'error_log_location',
  'export_error_log'
])

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await rawInvoke<T>(command, args)
  } catch (e) {
    if (!SILENT_COMMANDS.has(command)) {
      reportClientError('error', 'ipc', command, errorMessage(e))
    }
    throw e
  }
}

export const ipc = {
  openLogFile: (path: string, requestId: string) =>
    call<FileInfo>('open_log_file', { path, requestId }),

  /** Stops an import started with the same `requestId`; safe to call after it finished. */
  cancelOpenLog: (requestId: string) => call<void>('cancel_open_log', { requestId }),

  /** Subscribe to import progress. Remember to call the returned unlisten fn. */
  onOpenProgress: (handler: (progress: OpenProgress) => void) =>
    listen<OpenProgress>('log-open-progress', (event) => handler(event.payload)),

  adbStatus: () => call<AdbStatus>('adb_status'),
  listAdbDevices: () => call<AdbDevice[]>('list_adb_devices'),
  startDeviceStream: (options: DeviceCaptureOptions) =>
    call<FileInfo>('start_device_stream', { options: plain(options) }),
  pollDeviceStream: (sessionId: string) =>
    call<StreamStatus>('poll_device_stream', { sessionId }),
  stopDeviceStream: (sessionId: string) =>
    call<void>('stop_device_stream', { sessionId }),
  /** Force-release streams for a serial the webview no longer has a tab for. */
  releaseDeviceStream: (serial: string) =>
    call<void>('release_device_stream', { serial }),

  /** "查找"模式：只返回命中位置，不改变 queryLogs 的结果集。 */
  searchHits: (sessionId: string, filter: FilterSpec, limit: number) =>
    call<SearchHits>('search_hits', { sessionId, filter: plain(filter), limit }),

  queryLogs: (sessionId: string, filter: FilterSpec, offset: number, limit: number) =>
    call<QueryResult>('query_logs', {
      sessionId,
      filter: plain(filter),
      offset,
      limit
    }),

  /** 时间轴分布：命中条目在整份日志时间轴上的分桶计数。 */
  logTimeline: (sessionId: string, filter: FilterSpec, bins: number) =>
    call<TimeTimeline>('log_timeline', { sessionId, filter: plain(filter), bins }),

  closeSession: (sessionId: string) => call<void>('close_session', { sessionId }),

  exportFiltered: (sessionId: string, filter: FilterSpec, destPath: string) =>
    call<number>('export_filtered', {
      sessionId,
      filter: plain(filter),
      destPath
    }),

  listBuiltinTemplates: () => call<Rule[]>('list_builtin_templates'),
  listRuleBundle: () => call<RulesView>('list_rule_bundle'),
  rulesLocation: () => call<string>('rules_location'),
  saveCustomRule: (rule: Rule) => call<RulesView>('save_custom_rule', { rule: plain(rule) }),
  deleteCustomRule: (id: string) => call<RulesView>('delete_custom_rule', { id }),
  saveRuleCategory: (category: RuleCategory) =>
    call<RulesView>('save_rule_category', { category: plain(category) }),
  deleteRuleCategory: (id: string) => call<RulesView>('delete_rule_category', { id }),
  exportRulesToFilePath: (destPath: string) =>
    call<number>('export_rules_to_file', { destPath }),
  importRulesFromFilePath: (path: string) =>
    call<RulesView>('import_rules_from_file', { path }),
  exportRulesJson: () => call<string>('export_rules_json'),
  importRulesJson: (json: string) => call<RulesView>('import_rules_json', { json }),

  // --- search keyword history ---
  listSearchHistory: () => call<SearchHistoryEntry[]>('list_search_history'),
  rememberSearch: (keyword: string, mode: string, isRegex: boolean, caseSensitive: boolean) =>
    call<SearchHistoryEntry[]>('remember_search', { keyword, mode, isRegex, caseSensitive }),
  deleteSearchHistory: (index: number) =>
    call<SearchHistoryEntry[]>('delete_search_history', { index }),
  clearSearchHistory: () => call<SearchHistoryEntry[]>('clear_search_history'),

  listCapturePackages: (serial: string) =>
    call<string[]>('list_capture_packages', { serial }),
  probeNetworkCapture: (serial: string, packages: string[]) =>
    call<ProbeReport>('probe_network_capture', { serial, packages }),
  startCdpCapture: (options: CdpCaptureOptions) =>
    call<NetworkSessionInfo>('start_cdp_capture', { options: plain(options) }),
  startProxyCapture: (options: ProxyCaptureOptions) =>
    call<NetworkSessionInfo>('start_proxy_capture', { options: plain(options) }),
  pollNetworkCapture: (id: string) =>
    call<NetworkPollResult>('poll_network_capture', { id }),
  queryNetworkRequests: (id: string) =>
    call<NetworkRequest[]>('query_network_requests', { id }),
  clearNetworkCapture: (id: string) => call<void>('clear_network_capture', { id }),
  stopNetworkCapture: (id: string) => call<void>('stop_network_capture', { id }),
  closeNetworkCapture: (id: string) => call<void>('close_network_capture', { id }),
  importNetworkCapture: (path: string) =>
    call<NetworkSessionInfo>('import_network_capture', { path }),
  configureDeviceProxy: (serial: string, proxy: string) =>
    call<ProxyState>('configure_device_proxy', { serial, proxy }),
  restoreDeviceProxy: (serial: string) =>
    call<ProxyState>('restore_device_proxy', { serial }),

  // --- standalone proxy (no device involved) ---
  startProxyServer: (port: number) =>
    call<ProxyServerStatus>('start_proxy_server', { port }),
  stopProxyServer: () => call<void>('stop_proxy_server'),
  proxyServerStatus: () => call<ProxyServerStatus>('proxy_server_status'),
  pollProxyServer: () => call<NetworkRequest[]>('poll_proxy_server'),

  // --- client self-diagnostics ---
  listErrorLog: () => call<ErrorRecord[]>('list_error_log'),
  clearErrorLog: () => call<void>('clear_error_log'),
  errorLogLocation: () => call<string>('error_log_location'),
  exportErrorLog: (destPath: string) => call<number>('export_error_log', { destPath })
}

export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message
  try {
    return JSON.stringify(e)
  } catch {
    return String(e)
  }
}
