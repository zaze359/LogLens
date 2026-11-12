import { invoke } from '@tauri-apps/api/core'
import type {
  FileInfo,
  FilterSpec,
  QueryResult,
  Rule,
  RuleCategory,
  RulesView
} from '../types'

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/** Vue reactive proxies are flattened before crossing the IPC boundary. */
function plain<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

export const ipc = {
  openLogFile: (path: string) => invoke<FileInfo>('open_log_file', { path }),

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
