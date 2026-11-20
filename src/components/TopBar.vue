<script setup lang="ts">
import { computed } from 'vue'
import {
  openDevicePicker,
  openErrorLog,
  openJsonPanel,
  openProxyPanel,
  openUpdatePanel,
  pickAndLoadFile,
  store
} from '../stores/session'

const info = computed(() => store.fileInfo)

/** 顶栏版本号按钮的提示：有更新时直接说明新版本号。 */
const updateTitle = computed<string>(() => {
  const found = store.update.available
  if (found) return `发现新版本 ${found.version}，点击查看`
  return '查看版本并检查更新'
})

/** 独立代理是否在跑。状态来自 store，面板关着也照样更新。 */
const proxyLive = computed(() => !!store.proxyServer?.running)

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
      尚未载入日志 · 打开日志文件，或选择设备进入设备工作区
    </div>
    <div class="spacer" />
    <!-- 抓取的开始 / 停止在设备工作区的「日志」工具里，顶栏不再有脱离设备的抓取按钮。
         「清空筛选」「导出命中结果」作用于当前日志结果集，已移到日志面板的筛选栏，
         这样它们随面板一起显隐，而不是在顶栏常驻。 -->
    <button class="btn btn-sm" title="打开通用 JSON 格式化面板，可粘贴任意 JSON" @click="openJsonPanel()">
      JSON 格式化
    </button>
    <button
      class="btn btn-primary btn-sm"
      :disabled="!!store.importTask"
      :title="store.importTask ? '正在导入另一个日志' : '在新标签中打开日志文件（最多 4 个）'"
      @click="pickAndLoadFile"
    >
      {{ store.importTask ? '导入中…' : '打开日志文件' }}
    </button>

    <!--
      Two symmetric shortcuts, each creating a workspace tab:
      · 打开日志文件 → 本地文件工作区（多文件靠标签条切换）
      · 设备        → 选择设备，创建设备工作区（抓日志 / 抓包 / 后续 adb 功能）
    -->
    <template v-if="store.tauriReady">
      <div class="divider-v" />
      <button
        class="btn btn-sm"
        title="选择一台 Android 设备，打开设备工作区（抓日志 / 抓包 / adb）"
        @click="openDevicePicker"
      >
        设备
      </button>
      <!-- 代理状态常驻顶栏：开启后这里直接显示地址和来源数，不用打开面板才知道它在跑 -->
      <button
        class="btn btn-sm proxy-entry"
        :class="{ live: proxyLive }"
        :title="
          proxyLive
            ? `独立代理运行中：客户端把代理指向 ${store.proxyServer?.endpoint}（已接入 ${store.proxyServer?.sources ?? 0} 个来源）。点击查看`
            : '开启一个本地 HTTP 代理：任何把代理指向它的设备都会被接收，不依赖 adb'
        "
        @click="openProxyPanel"
      >
        <span v-if="proxyLive" class="live-dot" />
        代理<template v-if="proxyLive">
          <span class="mono proxy-entry-addr">{{ store.proxyServer?.endpoint }}</span>
        </template>
      </button>
      <button
        class="btn btn-sm error-log-entry"
        :class="{ 'has-new': store.errorLogUnread > 0 }"
        title="查看客户端异常与抓包流程记录"
        @click="openErrorLog"
      >
        诊断日志<span v-if="store.errorLogUnread" class="error-log-badge">{{
          store.errorLogUnread > 99 ? '99+' : store.errorLogUnread
        }}</span>
      </button>
      <!-- 版本号也是更新入口：发现新版本时带一个提示点，不额外占一个按钮位置 -->
      <button class="btn btn-ghost btn-sm version-entry" :title="updateTitle" @click="openUpdatePanel">
        v{{ store.appVersion || '—' }}
        <span v-if="store.update.available" class="update-dot" />
      </button>
    </template>
  </header>
</template>
