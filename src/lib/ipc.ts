import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  AdbDevice,
  AdbStatus,
  DeviceCaptureOptions,
  FileInfo,
  FilterSpec,
  OpenProgress,
  QueryResult,
  Rule,
  RuleCategory,
  RulesView,
  StreamStatus
} from '../types'

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/** Vue reactive proxies are flattened before crossing the IPC boundary. */
function plain<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

export const ipc = {
  openLogFile: (path: string, requestId: string) =>
    invoke<FileInfo>('open_log_file', { path, requestId }),

  /** Stops an import started with the same `requestId`; safe to call after it finished. */
  cancelOpenLog: (requestId: string) => invoke<void>('cancel_open_log', { requestId }),

  /** Subscribe to import progress. Remember to call the returned unlisten fn. */
  onOpenProgress: (handler: (progress: OpenProgress) => void) =>
    listen<OpenProgress>('log-open-progress', (event) => handler(event.payload)),

  adbStatus: () => invoke<AdbStatus>('adb_status'),
  listAdbDevices: () => invoke<AdbDevice[]>('list_adb_devices'),
  startDeviceStream: (options: DeviceCaptureOptions) =>
    invoke<FileInfo>('start_device_stream', { options: plain(options) }),
  pollDeviceStream: (sessionId: string) =>
    invoke<StreamStatus>('poll_device_stream', { sessionId }),
  stopDeviceStream: (sessionId: string) =>
    invoke<void>('stop_device_stream', { sessionId }),

  queryLogs: (sessionId: string, filter: FilterSpec, offset: number, limit: number) =>
    invoke<QueryResult>('query_logs', {
      sessionId,
      filter: plain(filter),
      offset,
      limit
    }),

  closeSession: (sessionId: string) => invoke<void>('close_session', { sessionId }),

  exportFiltered: (sessionId: string, filter: FilterSpec, destPath: string) =>
    invoke<number>('export_filtered', {
      sessionId,
      filter: plain(filter),
      destPath
    }),

  listBuiltinTemplates: () => invoke<Rule[]>('list_builtin_templates'),
  listRuleBundle: () => invoke<RulesView>('list_rule_bundle'),
  rulesLocation: () => invoke<string>('rules_location'),
  saveCustomRule: (rule: Rule) => invoke<RulesView>('save_custom_rule', { rule: plain(rule) }),
  deleteCustomRule: (id: string) => invoke<RulesView>('delete_custom_rule', { id }),
  saveRuleCategory: (category: RuleCategory) =>
    invoke<RulesView>('save_rule_category', { category: plain(category) }),
  deleteRuleCategory: (id: string) => invoke<RulesView>('delete_rule_category', { id }),
  exportRulesToFilePath: (destPath: string) =>
    invoke<number>('export_rules_to_file', { destPath }),
  importRulesFromFilePath: (path: string) =>
    invoke<RulesView>('import_rules_from_file', { path }),
  exportRulesJson: () => invoke<string>('export_rules_json'),
  importRulesJson: (json: string) => invoke<RulesView>('import_rules_json', { json })
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
