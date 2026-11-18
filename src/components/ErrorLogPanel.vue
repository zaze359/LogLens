<script setup lang="ts">
import { computed, ref } from 'vue'
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog'
import {
  clearErrorLog,
  closeErrorLog,
  exportErrorLogFile,
  refreshErrorLog,
  setNotice,
  store
} from '../stores/session'
import type { ErrorRecord } from '../types'

/** Empty string means "all"; chips are a filter, not a navigation. */
const levelFilter = ref('')
const sourceFilter = ref('')
const expanded = ref<Set<string>>(new Set())

const records = computed<ErrorRecord[]>(() =>
  store.errorLog.filter(
    (r) =>
      (!levelFilter.value || r.level === levelFilter.value) &&
      (!sourceFilter.value || r.source === sourceFilter.value)
  )
)

function time(ts: number): string {
  return new Date(ts).toLocaleString('zh-CN', { hour12: false })
}

/** `info` 是抓包流程快照，不是错误；在筛选按钮上用「流程」更直白。 */
function levelLabel(level: string): string {
  if (!level) return '全部'
  return level === 'info' ? '流程' : level
}

function sourceLabel(source: string): string {
  if (!source) return '全部'
  if (source === 'flow') return '流程'
  if (source === 'capture') return '抓包(历史)'
  return source
}

/** New modules can start emitting their own source without a UI change. */
const sourceOptions = computed<string[]>(() => {
  const options = ['', 'rust', 'frontend', 'flow', 'capture']
  for (const r of store.errorLog) {
    if (!options.includes(r.source)) options.push(r.source)
  }
  return options
})

function toggle(id: string): void {
  const next = new Set(expanded.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  expanded.value = next
}

async function clear(): Promise<void> {
  if (!store.errorLog.length) return
  // Use the Tauri dialog API, not window.confirm: Tauri overrides the native confirm with
  // an async promise, so `!window.confirm(...)` is always false AND a rejected permission
  // check lands in the global unhandledrejection handler — which writes the failure straight
  // back into this very log, making it impossible to clear. Await + catch keeps it out.
  let ok = false
  try {
    ok = await confirmDialog(`确定清空全部 ${store.errorLog.length} 条诊断记录？此操作不可撤销。`, {
      title: '清空诊断日志',
      kind: 'warning'
    })
  } catch {
    return
  }
  if (!ok) return
  await clearErrorLog()
}

async function copyVisible(): Promise<void> {
  if (!records.value.length) {
    setNotice('当前筛选下没有可复制的记录')
    return
  }
  try {
    const text = records.value
      .map((r) =>
        r.detail
          ? `[${time(r.ts)}] [${r.level}] [${r.source}] ${r.scope}\n${r.message}\n${r.detail}\n`
          : `[${time(r.ts)}] [${r.level}] [${r.source}] ${r.scope}\n${r.message}\n`
      )
      .join('\n')
    await navigator.clipboard.writeText(text)
    setNotice(`已复制 ${records.value.length} 条诊断记录`)
  } catch (e) {
    setNotice(`复制失败: ${e instanceof Error ? e.message : String(e)}`)
  }
}
</script>

<template>
  <div class="modal-mask" @click.self="closeErrorLog">
    <div class="modal error-log-modal">
      <div class="modal-head">
        <span>诊断日志</span>
        <span class="badge">{{ store.errorLog.length }} 条</span>
        <span v-if="store.errorLogUnread" class="badge badge-new">{{ store.errorLogUnread }} 新</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" title="刷新" :disabled="store.errorLogLoading" @click="refreshErrorLog">
          {{ store.errorLogLoading ? '刷新中…' : '刷新' }}
        </button>
        <button class="btn btn-ghost btn-sm" title="关闭" @click="closeErrorLog">×</button>
      </div>

      <div class="error-log-toolbar">
        <span class="error-log-label">级别</span>
        <button
          v-for="lv in ['', 'panic', 'error', 'warning', 'info']"
          :key="lv || 'all'"
          class="chip"
          :class="{ active: levelFilter === lv }"
          @click="levelFilter = lv"
        >
          {{ levelLabel(lv) }}
        </button>
        <span class="divider-v" />
        <span class="error-log-label">来源</span>
        <button
          v-for="src in sourceOptions"
          :key="src || 'all'"
          class="chip"
          :class="{ active: sourceFilter === src }"
          @click="sourceFilter = src"
        >
          {{ sourceLabel(src) }}
        </button>
        <div class="spacer" />
        <span class="muted error-log-count">{{ records.length }} / {{ store.errorLog.length }}</span>
      </div>

      <div class="modal-body error-log-body">
        <div v-if="!records.length" class="error-log-empty">
          <template v-if="store.errorLog.length">当前筛选下没有记录</template>
          <template v-else>
            暂无记录。客户端异常会自动记在这里；流程快照（info / 流程）覆盖启动、文件、设备、抓包、规则等，只含计数，不含 URL 或正文。
          </template>
        </div>

        <div v-for="r in records" :key="r.id" class="error-row" :class="`el-${r.level}`">
          <div class="error-row-head" @click="r.detail && toggle(r.id)">
            <span class="el-time mono">{{ time(r.ts) }}</span>
            <span class="el-level" :class="`el-lv-${r.level}`">{{ r.level }}</span>
            <span class="el-source">{{ r.source }}</span>
            <span class="el-scope mono">{{ r.scope }}</span>
            <span v-if="r.count > 1" class="el-count">×{{ r.count }}</span>
            <div class="spacer" />
            <span v-if="r.detail" class="el-caret" :class="{ open: expanded.has(r.id) }">▸</span>
          </div>
          <div class="error-message">{{ r.message }}</div>
          <pre v-if="r.detail && expanded.has(r.id)" class="error-detail mono">{{ r.detail }}</pre>
        </div>
      </div>

      <div class="modal-foot error-log-foot">
        <span class="muted mono error-log-path" :title="store.errorLogPath">
          {{ store.errorLogPath || '日志路径未知' }}
        </span>
        <div class="spacer" />
        <button class="btn btn-sm" :disabled="!records.length" @click="copyVisible">复制当前筛选</button>
        <button class="btn btn-sm" :disabled="!store.errorLog.length" @click="exportErrorLogFile">导出</button>
        <button class="btn btn-sm btn-confirm" :disabled="!store.errorLog.length" @click="clear">清空</button>
      </div>
    </div>
  </div>
</template>
