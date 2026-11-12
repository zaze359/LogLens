<script setup lang="ts">
import { computed } from 'vue'
import {
  exportFilteredFile,
  pickAndLoadFile,
  resetFilter,
  store
} from '../stores/session'

const info = computed(() => store.fileInfo)

const readableSize = computed(() => {
  const bytes = info.value?.sizeBytes ?? 0
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
})
</script>

<template>
  <header class="topbar">
    <div class="brand"><span class="brand-mark">L</span> LogLens</div>
    <div class="divider-v" />

    <template v-if="info">
      <div class="file-meta">
        <span class="file-name mono" :title="info.path">{{ info.fileName }}</span>
        <span>{{ readableSize }}</span>
        <span>{{ info.totalLines.toLocaleString() }} 行</span>
        <span>{{ info.entryCount.toLocaleString() }} 条</span>
        <span v-if="info.unparsedLines > 0" class="muted">未识别 {{ info.unparsedLines }}</span>
      </div>
    </template>
    <div v-else class="file-meta muted">尚未载入日志 · 支持 .log / .txt / .logcat 等文本文件</div>

    <div class="spacer" />
    <button class="btn btn-sm" @click="resetFilter">清空筛选</button>
    <button class="btn btn-sm" :disabled="!info" @click="exportFilteredFile">导出命中结果</button>
    <button class="btn btn-primary btn-sm" @click="pickAndLoadFile">打开日志文件</button>
  </header>
</template>
