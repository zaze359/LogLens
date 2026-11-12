<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { errorMessage, ipc } from '../lib/ipc'
import { closeDevicePanel, startDeviceStream, store } from '../stores/session'
import type { AdbDevice, AdbStatus, DeviceCaptureOptions } from '../types'

const status = ref<AdbStatus | null>(null)
const devices = ref<AdbDevice[]>([])
const serial = ref(store.lastCapture?.serial ?? '')
const buffers = ref<string[]>([...(store.lastCapture?.buffers ?? [])])
// Off by default: `adb logcat -c` wipes the device buffer, and the lines it destroys are
// often exactly the ones the user opened this dialog to look at.
const clearBefore = ref(store.lastCapture?.clearBefore ?? false)
const busy = ref(false)
const loadingDevices = ref(false)
const localError = ref('')

const BUFFERS = [
  { value: 'main', label: 'main', hint: '应用日志（绝大多数问题看这个）' },
  { value: 'system', label: 'system', hint: '系统框架日志（权限、ActivityManager）' },
  { value: 'crash', label: 'crash', hint: '崩溃缓冲区' },
  { value: 'events', label: 'events', hint: '系统事件（二进制 events 标签）' }
]

const selected = computed(() => devices.value.find((d) => d.serial === serial.value) || null)

const readyDevices = computed(() => devices.value.filter((d) => d.state === 'device'))

const canStart = computed(
  () => !!status.value?.available && !!selected.value && selected.value.state === 'device' && !busy.value
)

/** Ready devices first, then the rest — the picker should open on something usable. */
const orderedDevices = computed(() =>
  [...devices.value].sort((a, b) => {
    const rank = (d: AdbDevice) => (d.state === 'device' ? 0 : 1)
    return rank(a) - rank(b) || a.serial.localeCompare(b.serial)
  })
)

function deviceLabel(device: AdbDevice): string {
  const name = device.model || device.serial
  return device.state === 'device' ? `${name}（${device.serial}）` : `${name}（${device.serial}）· ${stateLabel(device.state)}`
}

function stateLabel(state: string): string {
  switch (state) {
    case 'device':
      return '已授权'
    case 'unauthorized':
      return '未授权'
    case 'offline':
      return '离线'
    case 'no permissions':
      return '无权限'
    case 'recovery':
      return 'recovery'
    case 'bootloader':
      return 'bootloader'
    default:
      return state || '未知'
  }
}

function toggleBuffer(value: string): void {
  buffers.value = buffers.value.includes(value)
    ? buffers.value.filter((b) => b !== value)
    : [...buffers.value, value]
}

async function refresh(): Promise<void> {
  localError.value = ''
  loadingDevices.value = true
  try {
    status.value = await ipc.adbStatus()
    if (!status.value.available) {
      devices.value = []
      serial.value = ''
      return
    }
    devices.value = await ipc.listAdbDevices()
    const keep = devices.value.some((d) => d.serial === serial.value)
    if (!keep) {
      const remembered = devices.value.find((d) => d.serial === store.lastCapture?.serial)
      serial.value =
        remembered?.serial ?? readyDevices.value[0]?.serial ?? devices.value[0]?.serial ?? ''
    }
  } catch (e) {
    localError.value = errorMessage(e)
  } finally {
    loadingDevices.value = false
  }
}

async function start(): Promise<void> {
  if (!canStart.value) return
  localError.value = ''
  busy.value = true
  const options: DeviceCaptureOptions = {
    serial: serial.value,
    buffers: [...buffers.value],
    clearBefore: clearBefore.value
  }
  const ok = await startDeviceStream(options)
  busy.value = false
  if (ok) closeDevicePanel()
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="modal-mask" @click.self="closeDevicePanel">
    <div class="modal device-modal">
      <div class="modal-head">
        <span>从 Android 设备实时抓取日志</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" title="关闭" @click="closeDevicePanel">×</button>
      </div>

      <div class="modal-body">
        <!-- adb 状态 -->
        <!-- First call may start the adb server and take a few seconds; without this the
             dialog body is empty and looks like the live option is missing. -->
        <div v-if="!status && !localError" class="adb-line muted">
          <span class="dot-pending" />
          正在检测 adb（首次调用会启动 adb server，可能需要几秒）…
        </div>
        <div v-else-if="status && !status.available" class="adb-missing">
          <p class="adb-missing-title">未检测到 adb</p>
          <p class="empty-hint">{{ status.error }}</p>
          <p class="empty-hint">
            安装 Android SDK Platform-Tools 后，把 <code class="mono">platform-tools</code>
            目录加入 PATH；或设置环境变量 <code class="mono">ANDROID_HOME</code> /
            <code class="mono">ADB_PATH</code> 指到 adb 所在目录再重启客户端。
          </p>
        </div>
        <div v-else-if="status" class="adb-line">
          <span class="dot-ok" />
          <span class="mono">{{ status.version || 'adb 已就绪' }}</span>
          <span class="muted mono ellipsis" :title="status.path">{{ status.path }}</span>
          <div class="spacer" />
          <button class="btn btn-sm" :disabled="loadingDevices" @click="refresh">
            {{ loadingDevices ? '刷新中…' : '刷新设备' }}
          </button>
        </div>

        <template v-if="status?.available">
          <p class="live-note">
            <span class="live-dot" />
            <span>
              开始后会持续跟随新日志，随时可停。启动瞬间会先显示设备缓冲区里已有的日志，
              所以「刚刚发生的问题」也看得到。
            </span>
          </p>

          <div class="form-row">
            <span class="form-label">设备</span>
            <select v-model="serial" class="select" style="flex: 1">
              <option v-if="orderedDevices.length === 0" value="">未发现已连接设备</option>
              <option v-for="device in orderedDevices" :key="device.serial" :value="device.serial">
                {{ deviceLabel(device) }}
              </option>
            </select>
          </div>

          <p v-if="devices.length === 0" class="empty-hint">
            没有检测到设备。请用数据线连接手机，打开「开发者选项 → USB 调试」，并在手机弹窗里点「允许」。
            用「无线调试」时先在终端执行 <code class="mono">adb connect &lt;ip&gt;:&lt;端口&gt;</code>，再回来刷新。
          </p>
          <p v-else-if="selected && selected.state === 'unauthorized'" class="empty-hint warn">
            手机还没有授权这台电脑：解锁手机，在「允许 USB 调试吗？」弹窗里点允许，然后刷新设备。
          </p>
          <p v-else-if="selected && selected.state === 'offline'" class="empty-hint warn">
            设备处于离线状态：重新插拔数据线，或执行 <code class="mono">adb reconnect</code> 后刷新。
          </p>
          <p v-else-if="selected && selected.state === 'no permissions'" class="empty-hint warn">
            当前用户没有访问该设备的权限（Linux 常见）：配置 udev 规则或改用 root 运行 adb server。
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
        </template>

        <p v-if="localError" class="empty-hint error-text">{{ localError }}</p>
        <p v-if="store.error" class="empty-hint error-text">{{ store.error }}</p>
      </div>

      <div class="modal-foot">
        <span class="empty-hint" style="margin-right: auto">
          日志通过本机 adb 直接读取，不上传任何数据
        </span>
        <button class="btn btn-sm" @click="closeDevicePanel">取消</button>
        <button class="btn btn-primary btn-sm" :disabled="!canStart" @click="start">
          {{ busy ? '连接中…' : '开始实时抓取' }}
        </button>
      </div>
    </div>
  </div>
</template>
