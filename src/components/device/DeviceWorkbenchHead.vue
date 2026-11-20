<script setup lang="ts">
import { computed, ref } from 'vue'
import { closeDeviceWorkspace, deviceBySerial, reconnectDevice } from '../../stores/session'

/**
 * 设备层的头部 —— 只管「这台设备连没连上、怎么连回来、怎么关掉工作区」。
 *
 * 它属于设备工作区本身，不属于任何一个工具：日志、抓包共用这一个头部。所以连接状态
 * 和重连入口只在这里出现一次，工具内部只展示自己的功能状态与开关（抓取中 / 抓包中、
 * 开始、停止），不再掺设备连接的事。连上了也可以什么都不做 —— 工具是否使用是另一回事。
 */
const props = defineProps<{ serial: string }>()

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

const reconnecting = ref(false)
async function reconnect(): Promise<void> {
  if (reconnecting.value) return
  reconnecting.value = true
  try {
    await reconnectDevice(props.serial)
  } finally {
    reconnecting.value = false
  }
}
</script>

<template>
  <header class="device-page-head">
    <span class="device-page-icon">▣</span>
    <span class="device-page-name">{{ deviceName }}</span>
    <span class="mono muted device-page-serial">{{ serial }}</span>
    <span class="device-page-state" :class="{ warn: !deviceReady }">{{ stateText }}</span>
    <div class="spacer" />
    <button
      v-if="!deviceReady"
      class="btn btn-sm"
      :disabled="reconnecting"
      title="重新检测这台设备（数据线 / 授权恢复后点这里）"
      @click="reconnect"
    >
      {{ reconnecting ? '重连中…' : '重连' }}
    </button>
    <button
      class="btn btn-ghost btn-sm"
      title="关闭设备工作区（连同它的日志一起关闭，并清空设备代理）"
      @click="closeDeviceWorkspace(serial)"
    >×</button>
  </header>
</template>
