<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { deviceBySerial, registerDeviceCleanup, store } from '../../stores/session'
import NetworkCaptureTab from './NetworkCaptureTab.vue'

/**
 * 抓包工具的内容区 —— 这里只有这一件事的状态与开关；设备连没连上、怎么重连属于设备层
 * (`DeviceWorkbenchHead.vue`)，两个工具共用那一个头部，不在这里重复。
 *
 * 切工具不卸载（`v-show`），正在跑的采集不会中断。
 *
 * Adding an adb function: an entry in `operations.ts`, a `<div v-if="isNetwork">`-style body
 * here, and — if it leaves state on the device — a `shutdown()` call in `shutdownOperations()`.
 */
const props = defineProps<{ serial: string }>()

const networkTab = ref<InstanceType<typeof NetworkCaptureTab> | null>(null)
const connected = computed(() => !!deviceBySerial(props.serial))
/** 采集是否在跑。adb 断开后它仍可能为 true —— 代理还在设备上生效，采集照常继续。 */
const capturing = computed(() => !!networkTab.value?.isCapturing)

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
    <div class="device-page-body">
      <!-- adb 断开 ≠ 抓包失效：已下发到设备的代理仍然生效，内存里的采集会话也还在轮询。
           所以这里只提示"不能再开始/停止"，绝不把仍在工作的采集面板换成一纸"不可用"。
           面板保持挂载，采集状态和会话 id 才不会丢。 -->
      <p v-if="!connected" class="empty-hint warn">
        <template v-if="capturing">
          adb 连接已断开。设备代理仍在生效，抓包会继续进行；但还原代理需要 adb，请重新连接设备以恢复设备网络。
        </template>
        <template v-else>
          设备未连接，无法开始抓包。在上方设备栏重新连接后即可开始。
        </template>
      </p>
      <NetworkCaptureTab
        ref="networkTab"
        :serial="serial"
        :connected="connected"
        :active="(store.deviceOps[serial] ?? 'log') === 'network'"
      />
    </div>
  </section>
</template>
