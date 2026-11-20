<script setup lang="ts">
import { computed } from 'vue'
import {
  activateTab,
  closeDeviceWorkspace,
  closeTab,
  deviceBySerial,
  MAX_TABS,
  showDeviceWorkspace,
  store
} from '../stores/session'
import type { SessionTab } from '../stores/session'

/** Live tabs keep counting in the background; the badge is the proof they still work. */
function badge(tab: SessionTab): string {
  if (tab.stream) return `${tab.info.entryCount.toLocaleString()} 条 · 接收中`
  if (tab.source === 'stream') return `${tab.info.entryCount.toLocaleString()} 条 · 已停止`
  return `${tab.info.totalLines.toLocaleString()} 行`
}

/**
 * 标签条上只有**工作区**：一个设备工作区（一台设备一个）、一个本地文件工作区（一个文件
 * 一个）。设备的日志不在这里 —— 它属于设备，在设备工作区左边的工具栏里（见 DeviceToolRail）。
 */
const fileTabs = computed<SessionTab[]>(() => store.tabs.filter((tab) => !tab.info.device))
</script>

<template>
  <div v-if="store.tabs.length || store.deviceWorkspaces.length" class="tabbar">
    <div
      v-for="serial in store.deviceWorkspaces"
      :key="serial"
      class="tab device-tab-strip"
      :class="{ on: store.deviceView && store.activeSerial === serial }"
    >
      <button
        class="tab-main"
        :title="`设备工作区：${deviceBySerial(serial)?.model || serial}（日志 / 抓包都是它的工具）`"
        @click="showDeviceWorkspace(serial)"
      >
        <span class="device-tab-icon">▣</span>
        <span class="tab-title">{{ deviceBySerial(serial)?.model || serial }}</span>
        <span class="tab-badge">设备</span>
      </button>
      <button
        class="tab-x"
        title="关闭设备工作区（连同它的日志一起关闭，并清空设备代理）"
        aria-label="关闭设备工作区"
        @click.stop="closeDeviceWorkspace(serial)"
      >×</button>
    </div>

    <div v-if="store.deviceWorkspaces.length && fileTabs.length" class="tab-sep" />

    <div
      v-for="tab in fileTabs"
      :key="tab.id"
      class="tab"
      :class="{ on: tab.id === store.activeId && !store.deviceView, live: !!tab.stream }"
    >
      <button
        class="tab-main"
        :title="`${tab.title}\n${tab.info.path}\n${badge(tab)}`"
        @click="activateTab(tab.id)"
      >
        <span v-if="tab.stream" class="live-dot" />
        <span class="tab-title">{{ tab.title }}</span>
        <span class="tab-badge">{{ badge(tab) }}</span>
      </button>
      <button
        class="tab-x"
        title="关闭标签"
        aria-label="关闭标签"
        @click.stop="closeTab(tab.id)"
      >×</button>
    </div>

    <div class="spacer" />
    <span v-if="store.tabs.length > 1" class="tab-hint">
      {{ store.tabs.length }} / {{ MAX_TABS }} 个日志
    </span>
    <span v-else-if="!store.tabs.length" class="tab-hint">打开日志文件会在这里新建工作区</span>
    <span v-else class="tab-hint">打开文件或选择设备会新建工作区</span>
  </div>
</template>
