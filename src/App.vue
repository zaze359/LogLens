<script setup lang="ts">
import { onBeforeUnmount, onMounted } from 'vue'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import TopBar from './components/TopBar.vue'
import RuleSidebar from './components/RuleSidebar.vue'
import FilterBar from './components/FilterBar.vue'
import StatsPanel from './components/StatsPanel.vue'
import LogTable from './components/LogTable.vue'
import LogDetail from './components/LogDetail.vue'
import { init, loadFile, store } from './stores/session'

let unlistenDrop: (() => void) | null = null

onMounted(async () => {
  await init()
  if (!store.tauriReady) return
  try {
    // Native drag & drop gives us real absolute paths (HTML5 File objects do not).
    unlistenDrop = await getCurrentWebview().onDragDropEvent((event: unknown) => {
      const payload = (event as { payload?: { type?: string; paths?: string[] } } | null)?.payload
      if (payload?.type === 'drop' && payload.paths && payload.paths.length > 0) {
        void loadFile(payload.paths[0])
      }
    })
  } catch {
    // Drag & drop is an enhancement; the file picker remains available.
  }
})

onBeforeUnmount(() => {
  unlistenDrop?.()
})
</script>

<template>
  <div class="app-shell">
    <TopBar />

    <div v-if="store.error" class="banner">
      <span>{{ store.error }}</span>
      <div class="spacer" />
      <button class="btn btn-ghost btn-sm" @click="store.error = ''">关闭</button>
    </div>
    <div v-else-if="!store.tauriReady" class="banner info">
      浏览器预览模式：界面可查看，本地文件读写与规则持久化请在 Tauri 客户端中运行（npm run tauri dev）。
    </div>

    <div class="workspace">
      <RuleSidebar />
      <div class="main-pane">
        <FilterBar />
        <StatsPanel />
        <LogTable />
        <div class="statusbar">
          <span>已加载 <b class="stat-num">{{ store.entries.length }}</b> / 命中
            <b class="stat-num">{{ store.result?.matched ?? 0 }}</b> / 总计
            <b class="stat-num">{{ store.result?.total ?? 0 }}</b></span>
          <span v-if="store.result">耗时 {{ store.result.elapsedMs }} ms</span>
          <span v-if="store.fileInfo">未识别行 {{ store.fileInfo.unparsedLines }}</span>
          <span v-if="store.loadingMore" class="muted">加载更多中…</span>
          <div class="spacer" />
          <span class="muted">点击行查看完整内容 · 支持拖入日志文件</span>
        </div>
      </div>
      <LogDetail v-if="store.selected" />
    </div>

    <div v-if="store.notice" class="toast">{{ store.notice }}</div>
  </div>
</template>
