<script setup lang="ts">
import { computed } from 'vue'
import { cancelImport, store } from '../stores/session'

const task = computed(() => store.importTask)

const indexing = computed(() => task.value?.phase === 'indexing')

/** Bytes drive the bar; the entry count is just something to read while it moves. */
const percent = computed(() => {
  const t = task.value
  if (!t || t.totalBytes <= 0) return 0
  return Math.min(100, Math.round((t.bytesRead / t.totalBytes) * 100))
})

/** Before the first progress event arrives the size is unknown, so the bar just sweeps. */
const determinate = computed(() => (task.value?.totalBytes ?? 0) > 0 && !indexing.value)

function readable(bytes: number): string {
  if (bytes <= 0) return '0 B'
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`
}
</script>

<template>
  <!-- Deliberately not a modal: a 200 MB import can take a while, and covering the window
       would just be a nicer-looking version of the freeze this replaces. -->
  <div v-if="task" class="import-card">
    <div class="import-head">
      <span class="spinner" />
      <span class="import-title">{{ indexing ? '正在建立索引' : '正在导入日志' }}</span>
      <div class="spacer" />
      <button class="btn btn-sm btn-ghost" :disabled="task.cancelling" @click="cancelImport">
        {{ task.cancelling ? '取消中…' : '取消' }}
      </button>
    </div>

    <div class="import-file mono" :title="task.path">{{ task.fileName }}</div>

    <div class="progress-track" :class="{ indeterminate: !determinate }">
      <div v-if="determinate" class="progress-fill" :style="{ width: `${percent}%` }" />
    </div>

    <div class="import-meta">
      <span v-if="determinate" class="import-percent">{{ percent }}%</span>
      <span v-if="task.totalBytes">
        {{ readable(task.bytesRead) }} / {{ readable(task.totalBytes) }}
      </span>
      <span v-else>读取中…</span>
      <div class="spacer" />
      <span>{{ task.entries.toLocaleString() }} 条</span>
    </div>

    <div class="import-note">可以继续浏览其它标签，导入完成后会自动切到新标签</div>
  </div>
</template>
