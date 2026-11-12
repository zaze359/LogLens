<script setup lang="ts">
import { activateTab, closeTab, MAX_TABS, store } from '../stores/session'

/** Live tabs keep counting in the background; the badge is the proof they still work. */
function badge(tab: (typeof store.tabs)[number]): string {
  if (tab.stream) return `${tab.info.entryCount.toLocaleString()} 条 · 接收中`
  if (tab.source === 'stream') return `${tab.info.entryCount.toLocaleString()} 条 · 已停止`
  return `${tab.info.totalLines.toLocaleString()} 行`
}
</script>

<template>
  <div v-if="store.tabs.length" class="tabbar">
    <button
      v-for="tab in store.tabs"
      :key="tab.id"
      class="tab"
      :class="{ on: tab.id === store.activeId, live: !!tab.stream }"
      :title="`${tab.title}\n${tab.info.path || tab.info.device}\n${badge(tab)}`"
      @click="activateTab(tab.id)"
    >
      <span v-if="tab.stream" class="live-dot" />
      <span class="tab-title">{{ tab.title }}</span>
      <span class="tab-badge">{{ badge(tab) }}</span>
      <span
        class="tab-x"
        role="button"
        title="关闭标签"
        @click.stop="closeTab(tab.id)"
      >×</span>
    </button>

    <div class="spacer" />
    <span v-if="store.tabs.length > 1" class="tab-hint">
      {{ store.tabs.length }} / {{ MAX_TABS }} 个标签
    </span>
    <span v-else class="tab-hint">打开文件或连接设备会新建标签</span>
  </div>
</template>
