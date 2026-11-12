<script setup lang="ts">
import { computed } from 'vue'
import {
  exportFilteredFile,
  openDevicePanel,
  pickAndLoadFile,
  resetFilter,
  stopDeviceStream,
  store
} from '../stores/session'

const info = computed(() => store.fileInfo)

const readableSize = computed(() => {
  const bytes = info.value?.sizeBytes ?? 0
  if (bytes <= 0) return ''
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
})

/** Where the current entries came from, shown next to the name. */
const sourceBadge = computed(() => {
  return info.value?.source === 'stream' ? '实时' : ''
})

const sourceTitle = computed(() => {
  if (!info.value) return ''
  if (info.value.source === 'file') return info.value.path
  const adb = info.value.streaming ? '正在通过 adb logcat 实时抓取' : '已结束的 adb logcat 实时抓取'
  return `${adb}（${info.value.device}）`
})
</script>

<template>
  <header class="topbar">
    <div class="brand"><span class="brand-mark">L</span> LogLens</div>
    <div class="divider-v" />

    <template v-if="info">
      <div class="file-meta">
        <span v-if="sourceBadge" class="source-badge" :class="{ live: info.source === 'stream' }">
          <span v-if="info.source === 'stream'" class="live-dot" />
          {{ sourceBadge }}
        </span>
        <span class="file-name mono" :title="sourceTitle">{{ info.fileName }}</span>
        <span v-if="readableSize">{{ readableSize }}</span>
        <span>{{ info.totalLines.toLocaleString() }} 行</span>
        <span>{{ info.entryCount.toLocaleString() }} 条</span>
        <span v-if="info.unparsedLines > 0" class="muted">未识别 {{ info.unparsedLines }}</span>
      </div>
    </template>
    <div v-else class="file-meta muted">
      尚未载入日志 · 打开本地文件，或连上 Android 设备实时抓取 logcat
    </div>

    <div class="spacer" />
    <button v-if="store.deviceStream" class="btn btn-sm btn-danger-ghost" @click="stopDeviceStream">
      停止实时抓取
    </button>
    <button class="btn btn-sm" @click="resetFilter">清空筛选</button>
    <button class="btn btn-sm" :disabled="!info" @click="exportFilteredFile">导出命中结果</button>
    <button
      class="btn btn-sm"
      title="连接 Android 设备，在新标签中实时抓取 logcat"
      @click="openDevicePanel()"
    >
      连接设备
    </button>
    <button
      class="btn btn-primary btn-sm"
      :disabled="!!store.importTask"
      :title="store.importTask ? '正在导入另一个日志' : '在新标签中打开日志文件（最多 4 个）'"
      @click="pickAndLoadFile"
    >
      {{ store.importTask ? '导入中…' : '打开日志文件' }}
    </button>
  </header>
</template>
