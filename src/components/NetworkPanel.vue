<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { errorMessage, ipc } from '../lib/ipc'
import { closeNetworkPanel } from '../stores/session'
import type {
  AdbDevice,
  AdbStatus,
  NetworkCaptureStatus,
  NetworkRequest,
  NetworkSessionInfo,
  ProbeReport,
  ProxyState
} from '../types'

const status = ref<AdbStatus | null>(null)
const devices = ref<AdbDevice[]>([])
const serial = ref('')
const report = ref<ProbeReport | null>(null)
const probing = ref(false)
const busy = ref(false)
const error = ref('')

const requests = ref<NetworkRequest[]>([])
const captureInfo = ref<NetworkSessionInfo | null>(null)
const captureStatus = ref<NetworkCaptureStatus | null>(null)
const selected = ref<NetworkRequest | null>(null)

const proxyInput = ref('')
const proxyState = ref<ProxyState | null>(null)
const showAdvanced = ref(false)

let timer: ReturnType<typeof setInterval> | null = null

const readyDevices = computed(() => devices.value.filter((d) => d.state === 'device'))
const cdpSocket = computed(() => report.value?.evidence.devtools[0]?.socket ?? '')
const canProbe = computed(() => !!status.value?.available && !!serial.value && !probing.value)
const canStartCdp = computed(
  () => !!serial.value && !!cdpSocket.value && !captureStatus.value?.running && !busy.value
)

function formatSize(bytes: number): string {
  if (bytes <= 0) return '—'
  if (bytes < 1024) return `${bytes} B`
  return `${(bytes / 1024).toFixed(1)} KB`
}

function statusClass(row: NetworkRequest): string {
  if (row.error) return 'net-fail'
  if (row.status >= 400) return 'net-error'
  if (row.status >= 300) return 'net-redirect'
  return 'net-ok'
}

async function refreshDevices(): Promise<void> {
  error.value = ''
  try {
    status.value = await ipc.adbStatus()
    devices.value = status.value.available ? await ipc.listAdbDevices() : []
    if (!devices.value.some((d) => d.serial === serial.value)) {
      serial.value = readyDevices.value[0]?.serial ?? devices.value[0]?.serial ?? ''
    }
  } catch (e) {
    error.value = errorMessage(e)
  }
}

function onDeviceChange(): void {
  report.value = null
  if (serial.value) void runProbe()
}

async function runProbe(): Promise<void> {
  if (!canProbe.value) return
  probing.value = true
  error.value = ''
  report.value = null
  try {
    // A full dumpsys per package is slow; two dozen covers the apps people actually debug.
    const packages = await ipc.listCapturePackages(serial.value)
    report.value = await ipc.probeNetworkCapture(serial.value, packages.slice(0, 24))
  } catch (e) {
    error.value = errorMessage(e)
  } finally {
    probing.value = false
  }
}

function stopPolling(): void {
  if (timer !== null) {
    clearInterval(timer)
    timer = null
  }
}

async function pollOnce(): Promise<void> {
  const info = captureInfo.value
  if (!info) return
  try {
    const result = await ipc.pollNetworkCapture(info.id)
    requests.value = result.requests
    captureStatus.value = result.status
    if (!result.status.running) stopPolling()
  } catch (e) {
    error.value = errorMessage(e)
    stopPolling()
  }
}

async function startCapture(): Promise<void> {
  if (!canStartCdp.value) return
  busy.value = true
  error.value = ''
  try {
    stopPolling()
    if (captureInfo.value) await ipc.closeNetworkCapture(captureInfo.value.id)
    const info = await ipc.startCdpCapture({ serial: serial.value, socket: cdpSocket.value, port: 0 })
    captureInfo.value = info
    requests.value = []
    selected.value = null
    captureStatus.value = {
      sessionId: info.id,
      running: true,
      entryCount: 0,
      events: 0,
      target: '',
      error: ''
    }
    timer = setInterval(() => void pollOnce(), 800)
  } catch (e) {
    error.value = errorMessage(e)
  } finally {
    busy.value = false
  }
}

async function stopCapture(): Promise<void> {
  stopPolling()
  const info = captureInfo.value
  if (!info) return
  try {
    await ipc.stopNetworkCapture(info.id)
  } catch (e) {
    error.value = errorMessage(e)
  }
  if (captureStatus.value) captureStatus.value.running = false
}

async function pickCaptureFile(): Promise<void> {
  error.value = ''
  try {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: '选择 HAR / JSONL 抓包文件',
      filters: [{ name: '抓包文件', extensions: ['har', 'json', 'jsonl', 'log', 'txt'] }]
    })
    if (typeof picked !== 'string' || picked.length === 0) return
    stopPolling()
    if (captureInfo.value) await ipc.closeNetworkCapture(captureInfo.value.id)
    const info = await ipc.importNetworkCapture(picked)
    captureInfo.value = info
    requests.value = await ipc.queryNetworkRequests(info.id)
    captureStatus.value = null
    selected.value = null
  } catch (e) {
    error.value = errorMessage(e)
  }
}

async function configureProxy(): Promise<void> {
  error.value = ''
  try {
    proxyState.value = await ipc.configureDeviceProxy(serial.value, proxyInput.value)
  } catch (e) {
    error.value = errorMessage(e)
  }
}

async function restoreProxy(): Promise<void> {
  error.value = ''
  try {
    proxyState.value = await ipc.restoreDeviceProxy(serial.value)
  } catch (e) {
    error.value = errorMessage(e)
  }
}

async function close(): Promise<void> {
  stopPolling()
  const info = captureInfo.value
  // Do not leave a CDP forward (or a global proxy) running behind a closed dialog.
  if (info) {
    try {
      await ipc.closeNetworkCapture(info.id)
    } catch {
      // The backend also cleans up on window close; a failed IPC here is not fatal.
    }
  }
  if (proxyState.value?.active) {
    try {
      await ipc.restoreDeviceProxy(serial.value)
    } catch {
      // ProxyGuard::drop is the final safety net.
    }
  }
  closeNetworkPanel()
}

onMounted(async () => {
  await refreshDevices()
  if (serial.value) await runProbe()
})

onBeforeUnmount(() => {
  stopPolling()
})
</script>

<template>
  <div class="modal-mask" @click.self="close">
    <div class="modal network-modal">
      <div class="modal-head">
        <span>设备网络请求</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" title="关闭" @click="close">×</button>
      </div>

      <div class="modal-body">
        <div v-if="status && !status.available" class="empty-hint warn">
          {{ status.error }}
        </div>

        <template v-else>
          <div class="form-row">
            <span class="form-label">设备</span>
            <select v-model="serial" class="select" style="flex: 1" @change="onDeviceChange">
              <option v-if="devices.length === 0" value="">未发现已连接设备</option>
              <option v-for="device in devices" :key="device.serial" :value="device.serial">
                {{ device.model || device.serial }}（{{ device.serial }}）· {{ device.state }}
              </option>
            </select>
            <button class="btn btn-sm" :disabled="!canProbe || busy" @click="runProbe">
              {{ probing ? '体检中…' : '运行环境体检' }}
            </button>
          </div>

          <!-- 体检结论 -->
          <div v-if="report" class="probe-summary">
            <div class="probe-summary-head">
              <span class="probe-badge" :class="report.recommended === 'cdp' ? 'ok' : 'muted'">
                推荐路线：{{ report.recommended }}
              </span>
              <span class="muted mono">
                Android {{ report.evidence.androidRelease }} / SDK {{ report.evidence.sdk }} ·
                {{ report.evidence.buildType }} ·
                {{ report.evidence.rootShell ? '有 root' : '无 root' }}
              </span>
            </div>
            <p>{{ report.summary }}</p>
          </div>

          <div v-if="report" class="route-list">
            <div
              v-for="route in report.routes"
              :key="route.id"
              class="route-card"
              :class="{ off: !route.available, recommended: route.id === report.recommended }"
            >
              <div class="route-head">
                <span class="route-state" :class="route.available ? 'on' : 'off'">
                  {{ route.available ? '可用' : '不可用' }}
                </span>
                <b>{{ route.label }}</b>
                <span v-if="route.httpsPlaintext" class="route-tag">HTTPS 明文</span>
                <div class="spacer" />
                <span class="muted">{{ route.coverage }}</span>
              </div>
              <p class="route-reason">{{ route.reason }}</p>
              <ol v-if="route.available && route.id === report.recommended" class="route-steps">
                <li v-for="(step, i) in route.steps" :key="i">{{ step }}</li>
              </ol>
            </div>
          </div>

          <!-- 采集控制 -->
          <div class="net-controls">
            <button class="btn btn-primary btn-sm" :disabled="!canStartCdp" @click="startCapture">
              {{ busy ? '连接中…' : '开始 WebView 采集' }}
            </button>
            <button class="btn btn-sm" :disabled="!captureStatus?.running" @click="stopCapture">
              停止采集
            </button>
            <button class="btn btn-sm" @click="pickCaptureFile">导入 HAR / JSONL</button>
            <div class="spacer" />
            <span v-if="captureStatus?.running" class="live-hint">
              <span class="live-dot" />采集中 · {{ captureStatus.events }} 个事件
            </span>
            <span v-if="captureInfo" class="muted">{{ captureInfo.label }} · {{ requests.length }} 条</span>
          </div>

          <p v-if="report && report.evidence.devtools.length && !report.evidence.devtools[0].pages" class="empty-hint warn">
            调试通道已就绪，但当前没有打开的 H5 页面。请在 App 里进入一个 WebView 页面，请求会自动出现在这里。
          </p>
          <p v-if="captureStatus?.target" class="empty-hint mono">
            当前页面：{{ captureStatus.target }}
          </p>
          <p v-if="captureStatus?.error" class="empty-hint error-text">{{ captureStatus.error }}</p>
          <p v-if="error" class="empty-hint error-text">{{ error }}</p>

          <!-- 请求列表 -->
          <div class="net-table-wrap">
            <table class="net-table">
              <thead>
                <tr>
                  <th style="width: 96px">时间</th>
                  <th style="width: 62px">方法</th>
                  <th>URL</th>
                  <th style="width: 66px">状态</th>
                  <th style="width: 76px">耗时</th>
                  <th style="width: 80px">大小</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="row in requests"
                  :key="row.index"
                  :class="{ picked: selected?.index === row.index }"
                  @click="selected = row"
                >
                  <td class="mono">{{ row.timeKey || row.startedAt || '—' }}</td>
                  <td class="mono">{{ row.method }}</td>
                  <td class="mono net-url" :title="row.url">{{ row.url }}</td>
                  <td class="mono" :class="statusClass(row)">
                    {{ row.status || row.error || '—' }}
                  </td>
                  <td class="mono">{{ row.durationMs }} ms</td>
                  <td class="mono">{{ formatSize(row.responseSize) }}</td>
                </tr>
                <tr v-if="requests.length === 0">
                  <td colspan="6" class="net-empty">
                    {{ captureStatus?.running ? '等待 WebView 请求…（在 App 里打开一个 H5 页面）' : '暂无请求。可以先开始采集，或导入一份 HAR / JSONL 抓包文件。' }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          <!-- 选中请求详情 -->
          <div v-if="selected" class="net-detail">
            <div class="net-detail-head">
              <b class="mono">{{ selected.method }} {{ selected.url }}</b>
              <div class="spacer" />
              <span v-if="selected.redacted" class="route-tag">敏感字段已隐藏</span>
              <button class="btn btn-ghost btn-sm" @click="selected = null">×</button>
            </div>
            <div class="net-detail-grid">
              <div>
                <p class="net-detail-title">请求头</p>
                <p v-for="(h, i) in selected.requestHeaders" :key="i" class="mono">
                  {{ h.name }}: {{ h.value }}
                </p>
                <p v-if="!selected.requestHeaders.length" class="muted">—</p>
                <p v-if="selected.requestBody" class="net-detail-title">请求体</p>
                <pre v-if="selected.requestBody" class="mono">{{ selected.requestBody }}</pre>
              </div>
              <div>
                <p class="net-detail-title">响应头</p>
                <p v-for="(h, i) in selected.responseHeaders" :key="i" class="mono">
                  {{ h.name }}: {{ h.value }}
                </p>
                <p v-if="!selected.responseHeaders.length" class="muted">—</p>
                <p v-if="selected.responseBody" class="net-detail-title">响应体</p>
                <pre v-if="selected.responseBody" class="mono">{{ selected.responseBody }}</pre>
              </div>
            </div>
          </div>

          <!-- MITM 代理：高级，可还原 -->
          <div class="net-advanced">
            <button class="btn btn-ghost btn-sm" @click="showAdvanced = !showAdvanced">
              {{ showAdvanced ? '收起' : '展开' }} · MITM 代理设置（原生接口路线）
            </button>
            <div v-if="showAdvanced" class="net-advanced-body">
              <div class="form-row">
                <span class="form-label">代理</span>
                <input v-model="proxyInput" class="select mono" style="flex: 1" placeholder="例如 10.0.0.2:8080" />
                <button class="btn btn-sm" :disabled="!serial || !proxyInput" @click="configureProxy">设置</button>
                <button class="btn btn-sm" :disabled="!proxyState?.active" @click="restoreProxy">还原</button>
              </div>
              <p class="empty-hint">
                全局代理只覆盖走系统代理的流量；Android 7+ 的 release 包不信任用户 CA，HTTPS 明文仍取决于证书条件。
                设置前会记录原值，点「还原」或关闭面板、退出应用时都会自动恢复，避免设备断网。
              </p>
              <p v-if="proxyState?.active" class="empty-hint warn mono">
                当前生效：{{ proxyState.proxy }}（原值：{{ proxyState.original || '无' }}）
              </p>
            </div>
          </div>
        </template>
      </div>

      <div class="modal-foot">
        <span class="empty-hint" style="margin-right: auto">
          请求与响应中的 token / Cookie 在导入时即脱敏，不会进入导出文件
        </span>
        <button class="btn btn-sm" @click="close">关闭</button>
      </div>
    </div>
  </div>
</template>
