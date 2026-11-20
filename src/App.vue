<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from 'vue'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import TopBar from './components/TopBar.vue'
import SessionTabs from './components/SessionTabs.vue'
import LogPane from './components/LogPane.vue'
import DevicePage from './components/device/DevicePage.vue'
import DeviceToolRail from './components/device/DeviceToolRail.vue'
import DeviceWorkbenchHead from './components/device/DeviceWorkbenchHead.vue'
import DeviceCaptureBar from './components/device/DeviceCaptureBar.vue'
import DevicePickerDialog from './components/device/DevicePickerDialog.vue'
import LogCaptureTab from './components/device/LogCaptureTab.vue'
import ImportOverlay from './components/ImportOverlay.vue'
import ErrorLogPanel from './components/ErrorLogPanel.vue'
import JsonFormatPanel from './components/JsonFormatPanel.vue'
import ProxyServerPanel from './components/ProxyServerPanel.vue'
import UpdatePanel from './components/UpdatePanel.vue'
import { init, loadFile, store } from './stores/session'

let unlistenDrop: (() => void) | null = null

/** 当前设备里选中的工具；不在设备工作区时为空。 */
const currentOp = computed<string>(() =>
  store.deviceView && store.activeSerial ? store.deviceOps[store.activeSerial] ?? 'log' : ''
)

/**
 * 日志面板什么时候可见：本地文件工作区一直在看它；设备工作区里只有选中「日志」工具、
 * 且这台设备已经有日志时看它。隐藏时只是 `v-show`，实时流与滚动位置都留着。
 */
const logVisible = computed<boolean>(() => {
  if (!store.deviceView) return true
  return currentOp.value === 'log' && !needCapture.value
})

const deviceHasLog = computed<boolean>(() =>
  store.tabs.some((tab) => tab.info.device === store.activeSerial)
)

/** 选了「日志」工具但这台设备还没有日志：显示「抓一份」的入口。 */
const needCapture = computed<boolean>(
  () =>
    store.deviceView &&
    !!store.activeSerial &&
    currentOp.value === 'log' &&
    !deviceHasLog.value
)

const deviceReady = computed<boolean>(
  () => store.devices.find((d) => d.serial === store.activeSerial)?.state === 'device'
)

/** 设备页只在选中非「日志」工具时显示；日志工具的内容是共用的日志面板。 */
function deviceToolVisible(serial: string): boolean {
  if (!store.deviceView || store.activeSerial !== serial) return false
  return (store.deviceOps[serial] ?? 'log') !== 'log'
}

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
    <SessionTabs />

    <div v-if="store.error" class="banner">
      <span>{{ store.error }}</span>
      <div class="spacer" />
      <button class="btn btn-ghost btn-sm" @click="store.error = ''">关闭</button>
    </div>
    <div v-else-if="!store.tauriReady" class="banner info">
      浏览器预览模式：界面可查看，本地文件读写、设备抓取与规则持久化请在 Tauri 客户端中运行（npm run tauri dev）。
    </div>

    <!--
      工作区内容。设备工作区左边多一条工具栏：日志 / 抓包都是这台设备的工具，不是另一个
      并列的工作区，所以关掉设备时它们一起关闭。
    -->
    <div class="app-body">
      <DeviceToolRail
        v-if="store.deviceView && store.activeSerial"
        :serial="store.activeSerial"
      />

      <div class="app-body-main">
        <!-- 设备层头部：连接状态与重连只在这里出现，两个工具共用；切工具不影响它 -->
        <DeviceWorkbenchHead
          v-if="store.deviceView && store.activeSerial"
          :serial="store.activeSerial"
        />

        <!-- 采集状态条：抓取是这台设备的事，状态和停止入口都留在「日志」工具里 -->
        <DeviceCaptureBar v-show="store.deviceView && logVisible" :serial="store.activeSerial" />

        <!-- 日志面板只有一份实例，两种来源共用 -->
        <LogPane v-show="logVisible" />

        <div v-show="needCapture" class="capture-empty">
          <div class="capture-empty-card">
            <p class="capture-empty-title">这台设备还没有日志</p>
            <LogCaptureTab :serial="store.activeSerial" :device-ready="deviceReady" />
          </div>
        </div>

        <!-- 每台设备一个页面，常驻挂载（v-show 隐藏），切换工具不会打断正在跑的采集 -->
        <DevicePage
          v-for="serial in store.deviceWorkspaces"
          :key="serial"
          v-show="deviceToolVisible(serial)"
          :serial="serial"
        />
      </div>
    </div>

    <DevicePickerDialog v-if="store.devicePicker" />

    <ErrorLogPanel v-if="store.errorLogOpen" />

    <JsonFormatPanel v-if="store.jsonPanel.open" />

    <!-- 独立代理不属于任何设备，所以挂在应用层，而不是设备工作区里 -->
    <ProxyServerPanel v-if="store.proxyPanelOpen" />

    <UpdatePanel v-if="store.updateOpen" />

    <Transition name="fade">
      <ImportOverlay v-if="store.importTask?.visible" />
    </Transition>

    <div v-if="store.notice" class="toast">{{ store.notice }}</div>
  </div>
</template>
