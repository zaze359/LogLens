<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import {
  closeDeviceWorkspace,
  deviceBySerial,
  openDevicePicker,
  registerDeviceCleanup,
  store
} from '../../stores/session'
import NetworkCaptureTab from './NetworkCaptureTab.vue'

/**
 * 设备工作区里**非日志工具**的内容区（今天是抓包）。日志工具用的是共用日志面板，所以
 * 不在这里渲染 —— 见 `LogPane.vue`。
 *
 * 功能切换在左侧工具栏（`DeviceToolRail.vue`），所以这个组件只负责内容与它自己的清理：
 * 切工具不卸载（`v-show`），正在跑的采集不会中断。
 *
 * Adding an adb function: an entry in `operations.ts`, a `<div v-if="isNetwork">`-style body
 * here, and — if it leaves state on the device — a `shutdown()` call in `shutdownOperations()`.
 */
const props = defineProps<{ serial: string }>()

const networkTab = ref<InstanceType<typeof NetworkCaptureTab> | null>(null)

const device = computed(() => deviceBySerial(props.serial))
const deviceReady = computed(() => device.value?.state === 'device')
const deviceName = computed(() => device.value?.model || props.serial)

const stateText = computed(() => {
  const d = device.value
  if (!d) return '已断开'
  if (d.state === 'device') return '已授权'
  if (d.state === 'unauthorized') return '未授权'
  if (d.state === 'offline') return '离线'
  if (d.state === 'no permissions') return '无权限'
  return d.state
})

/**
 * Every function's cleanup, in one place. Today only the network tab holds device-side state
 * (a global proxy and a port forward); a future "install APK" / "shell" tool would add its
 * own `shutdown()` here, so closing the workspace can never leave the device configured.
 */
async function shutdownOperations(): Promise<void> {
  await networkTab.value?.shutdown()
}

// The tab strip closes this workspace from outside the component, so cleanup is registered.
onMounted(() => registerDeviceCleanup(props.serial, shutdownOperations))
onBeforeUnmount(() => {
  registerDeviceCleanup(props.serial, null)
  // Unmounted without going through the tab strip (window close): best effort only, the
  // Rust-side guards and the proxy journal are the real guarantee.
  void shutdownOperations()
})
</script>

<template>
  <section class="device-page">
    <header class="device-page-head">
      <span class="device-page-icon">▣</span>
      <span class="device-page-name">{{ deviceName }}</span>
      <span class="mono muted device-page-serial">{{ serial }}</span>
      <span class="device-page-state" :class="{ warn: !deviceReady }">{{ stateText }}</span>
      <div class="spacer" />
      <button class="btn btn-ghost btn-sm" title="连接或切换到另一台设备" @click="openDevicePicker">
        换设备
      </button>
      <button
        class="btn btn-ghost btn-sm"
        title="关闭设备工作区（连同它的日志一起关闭，并还原代理）"
        @click="closeDeviceWorkspace(serial)"
      >×</button>
    </header>

    <div class="device-page-body">
      <p v-if="!device" class="empty-hint warn">
        这台设备已断开。点右上角「换设备」重新连接，或关闭本工作区。
      </p>

      <NetworkCaptureTab
        v-else
        ref="networkTab"
        :serial="serial"
        :active="(store.deviceOps[serial] ?? 'log') === 'network'"
      />
    </div>
  </section>
</template>
