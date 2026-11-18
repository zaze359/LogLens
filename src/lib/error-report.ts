/**
 * Frontend self-diagnostics.
 *
 * Every uncaught JS error, rejected promise and Vue render error is forwarded to the
 * Rust error log. Kept separate from `ipc.ts` on purpose: `ipc.ts` imports this module
 * to record failed commands, so importing back would create a cycle.
 *
 * `reportClientError` is deliberately fire-and-forget. A broken diagnostics channel
 * must never mask — or delay — the error that triggered the report.
 */

import { invoke } from '@tauri-apps/api/core'
import type { App } from 'vue'

/**
 * Browser-generated conditions that are expected noise rather than app failures.
 *
 * When a ResizeObserver callback changes layout, the browser defers the remaining
 * observations to the next frame and reports a bare `error` event. The observer
 * still converges correctly, so persisting it only buries real failures in the
 * support log. Every major error reporter ignores these for the same reason.
 */
const BENIGN_WINDOW_ERRORS = [
  'ResizeObserver loop completed with undelivered notifications.',
  // Older Blink wording for the same condition.
  'ResizeObserver loop limit exceeded.'
]

function isBenignWindowError(message: string): boolean {
  return BENIGN_WINDOW_ERRORS.some((known) => message.includes(known))
}

function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

function textOf(e: unknown): string {
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message
  try {
    return JSON.stringify(e)
  } catch {
    return String(e)
  }
}

/** Send one client-side failure to the bounded on-disk log. Safe before Tauri is ready. */
export function reportClientError(
  level: string,
  source: string,
  scope: string,
  message: string,
  detail = ''
): void {
  if (!inTauri()) return
  // Raw invoke, never the wrapped `call` in ipc.ts — otherwise a failure to report
  // would itself be reported, recursing until the stack blows.
  void invoke<void>('report_client_error', { level, source, scope, message, detail })
    .then(() => {
      // Notify only AFTER Rust has persisted the record — dispatching immediately would
      // race the store's refresh and leave the badge one error behind. No import of the
      // store here: that would cycle back through ipc.ts.
      window.dispatchEvent(new Event('loglens-client-error'))
    })
    .catch(() => {})
}

/** Install the three global collectors once, right after `createApp` and before `mount`. */
export function installGlobalErrorHandlers(app: App): void {
  if (typeof window === 'undefined') return

  // Vue render / lifecycle / watcher errors. Vue already logs to console; we also
  // persist them because a bundled app has no console for the user to read.
  app.config.errorHandler = (err: unknown, _instance: unknown, info: unknown) => {
    reportClientError(
      'error',
      'frontend',
      'vue.errorHandler',
      textOf(err),
      typeof info === 'string' ? `阶段: ${info}` : ''
    )
  }

  window.addEventListener('error', (event: ErrorEvent) => {
    // Resource load failures (img/script) have no Error object but still matter.
    const message = event.message || `资源加载失败: ${event.filename || '未知资源'}`
    if (isBenignWindowError(message)) return
    const detail = event.error instanceof Error ? event.error.stack || '' : ''
    reportClientError('error', 'frontend', 'window.onerror', message, detail)
  })

  window.addEventListener('unhandledrejection', (event: PromiseRejectionEvent) => {
    const reason: unknown = event.reason
    const detail =
      reason instanceof Error
        ? reason.stack || ''
        : typeof reason === 'object' && reason !== null
          ? JSON.stringify(reason)
          : ''
    reportClientError('error', 'frontend', 'unhandledrejection', textOf(reason), detail)
  })
}
