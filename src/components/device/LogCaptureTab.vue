<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { startDeviceStream, store } from '../../stores/session'
import type { DeviceCaptureOptions } from '../../types'

/**
 * Device operation: follow `adb logcat` live.
 *
 * Each operation owns its own controls and its own start action, so a new device operation
 * only has to be dropped into `DevicePage.vue`'s `OPERATIONS` list — the page never needs
 * to know what the operation does.
 *
 * The capture itself is a log tab under this device's workspace tab; this panel only starts
 * it, and says so rather than offering a second `adb logcat` the store would reject.
 */
const props = defineProps<{ serial: string; deviceReady: boolean }>()
const emit = defineEmits<{ started: [] }>()

const buffers = ref<string[]>([])
// 默认关：`adb logcat -c` 会清空设备缓冲区，而那批行往往正是用户打开面板想看的。
const clearBefore = ref(false)
// 本组件常驻挂载，serial 会随设备切换而变：每次换设备都重载它自己的偏好，
// 否则会把上一台设备的缓冲区设置带到新设备的「抓一份」卡片上。
watch(
  () => props.serial,
  (serial) => {
    buffers.value = [...(store.capturePrefs[serial]?.buffers ?? [])]
    clearBefore.value = store.capturePrefs[serial]?.clearBefore ?? false
  },
  { immediate: true }
)
const busy = ref(false)

const BUFFERS = [
  { value: 'main', label: 'main', hint: '应用日志（绝大多数问题看这个）' },
  { value: 'system', label: 'system', hint: '系统框架日志（权限、ActivityManager）' },
  { value: 'crash', label: 'crash', hint: '崩溃缓冲区' },
  { value: 'events', label: 'events', hint: '系统事件（二进制 events 标签）' }
]

/**
 * The capture itself is a log session shown by the shared log panel (the device's「日志」tool)
 * — this card only starts it. Knowing whether one is already running lets it say so instead
 * of offering a second `adb logcat` the store would reject anyway.
 */
const liveTab = computed(
  () => store.tabs.find((t) => t.stream?.serial === props.serial && t.stream.running) ?? null
)

const canStart = computed(
  () => props.deviceReady && !!props.serial && !busy.value && !liveTab.value
)

function toggleBuffer(value: string): void {
  buffers.value = buffers.value.includes(value)
    ? buffers.value.filter((b) => b !== value)
    : [...buffers.value, value]
}

async function start(): Promise<void> {
  if (!canStart.value) return
  busy.value = true
  const options: DeviceCaptureOptions = {
    serial: props.serial,
    buffers: [...buffers.value],
    clearBefore: clearBefore.value
  }
  const ok = await startDeviceStream(options)
  busy.value = false
  // The live log lands in the main window, so the dialog closes on success.
  if (ok) emit('started')
}
</script>

<template>
  <div class="device-tab-body">
    <p v-if="liveTab" class="live-note">
      <span class="live-dot" />
      <span>
        正在这台设备的「日志」工具里实时抓取。要停止就点顶栏的「停止实时抓取」，停完再回到这里
        可以改缓冲区重新开始。
      </span>
    </p>
    <p v-else class="live-note">
      <span class="live-dot" />
      <span>
        开始后日志会显示在这台设备的「日志」工具里；切到「抓包」再切回来它会继续接收。
        启动瞬间会先显示设备缓冲区里已有的日志，所以「刚刚发生的问题」也看得到。
      </span>
    </p>

    <div class="form-row">
      <span class="form-label">缓冲区</span>
      <div class="chip-row">
        <button
          v-for="buffer in BUFFERS"
          :key="buffer.value"
          class="chip"
          :class="{ on: buffers.includes(buffer.value) }"
          :title="buffer.hint"
          @click="toggleBuffer(buffer.value)"
        >
          {{ buffer.label }}
        </button>
      </div>
    </div>
    <p class="empty-hint" style="margin-top: -6px">
      不选 = adb 默认（main + system + crash）。排查崩溃选 crash，排查系统行为加 system。
    </p>

    <label class="check-line">
      <input v-model="clearBefore" type="checkbox" />
      <span>
        先清空设备缓冲区（<code class="mono">adb logcat -c</code>）
        <span class="muted">— 复现前清空，日志更干净；清空后无法恢复旧日志</span>
      </span>
    </label>

    <p v-if="store.error" class="empty-hint error-text">{{ store.error }}</p>

    <div class="device-op-actions">
      <span v-if="!deviceReady" class="empty-hint" style="margin-right: auto">
        {{ serial ? '这台设备当前不可用（未授权 / 离线）' : '先选择一台设备' }}
      </span>
      <button
        class="btn btn-primary btn-sm"
        :disabled="!canStart"
        :title="liveTab ? '这台设备已经在实时抓取，先停止再重新开始' : ''"
        @click="start"
      >
        {{ busy ? '连接中…' : liveTab ? '已在实时抓取' : '开始实时抓取' }}
      </button>
    </div>
  </div>
</template>
