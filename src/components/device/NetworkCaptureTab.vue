<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { toCurl } from '../../lib/curl'
import { errorMessage, ipc } from '../../lib/ipc'
import type {
  NetworkCaptureStatus,
  NetworkRequest,
  NetworkSessionInfo,
  ProbeReport,
  ProxyState
} from '../../types'

/**
 * The app already knows which device is active (`store.activeSerial`) — this tab never asks
 * for one, it only consumes the serial it is handed. `active` gates the (slow) probe, so
 * opening the log tab does not run `dumpsys` for nothing. The component stays mounted
 * (v-show) so switching tabs never interrupts a running capture.
 */
const props = defineProps<{ serial: string; active: boolean }>()

const report = ref<ProbeReport | null>(null)
const probing = ref(false)
const busy = ref(false)
const error = ref('')

/** WebView and native captures may run together and are merged into one request table. */
const captures = ref<NetworkSessionInfo[]>([])
const statuses = ref<Record<string, NetworkCaptureStatus>>({})
const requests = ref<NetworkRequest[]>([])
const fileMode = ref(false)
const selected = ref<NetworkRequest | null>(null)
const curlCopied = ref(false)
let curlTimer: ReturnType<typeof setTimeout> | null = null

const proxyInput = ref('')
const proxyState = ref<ProxyState | null>(null)
const showAdvanced = ref(false)
/**
 * The dock is short, so the probe verdict starts as a single line and the four-route
 * reasoning is opt-in. It collapses itself once there is something to watch.
 */
const showReport = ref(true)

/**
 * The device that currently owns the proxy / CDP forward. The dialog can switch its picker
 * while a capture is live, so cleanup must target the device the work was started on, not
 * whatever `props.serial` happens to be at teardown time — restoring the wrong device leaves
 * the real one pointing at a dead port.
 */
const boundSerial = ref('')
const restoreTarget = computed(() => boundSerial.value || props.serial)

/** Empty for CONNECT rows (undecrypted tunnels), which hide the copy button. */
const curlText = computed(() => (selected.value ? toCurl(selected.value) : ''))

let timer: ReturnType<typeof setInterval> | null = null

const cdpSocket = computed(() => report.value?.evidence.devtools[0]?.socket ?? '')
const canProbe = computed(() => !!props.serial && !probing.value)
const cdpRunning = computed(() =>
  captures.value.some((c) => c.source === 'cdp' && statuses.value[c.id]?.running)
)
const proxyRunning = computed(() =>
  captures.value.some((c) => c.source === 'proxy' && statuses.value[c.id]?.running)
)
const running = computed(() => Object.values(statuses.value).some((s) => s.running))
const events = computed(() =>
  Object.values(statuses.value).reduce((sum, s) => sum + s.events, 0)
)
const targets = computed(() =>
  Object.values(statuses.value)
    .map((s) => s.target)
    .filter(Boolean)
    .join(' · ')
)
const canStartCdp = computed(
  () => !!props.serial && !!cdpSocket.value && !cdpRunning.value && !busy.value
)
const canStartProxy = computed(
  () => !!props.serial && !proxyRunning.value && !busy.value && !proxyState.value?.active
)

function formatSize(bytes: number): string {
  if (bytes <= 0) return '—'
  if (bytes < 1024) return `${bytes} B`
  return `${(bytes / 1024).toFixed(1)} KB`
}

function statusText(row: NetworkRequest): string {
  return String(row.status || row.error || row.statusText || '—')
}

function statusClass(row: NetworkRequest): string {
  if (row.error) return 'net-fail'
  if (row.method === 'CONNECT' && row.status === 0) return 'net-tunnel'
  if (row.status >= 400) return 'net-error'
  if (row.status >= 300) return 'net-redirect'
  return 'net-ok'
}

function sourceLabel(row: NetworkRequest): string {
  if (row.source === 'cdp') return 'H5'
  if (row.source === 'proxy') return '原生'
  return '文件'
}

/** Re-running the probe is manual (it is slow), but switching devices invalidates it. */
async function runProbe(): Promise<void> {
  if (!canProbe.value) return
  probing.value = true
  error.value = ''
  report.value = null
  try {
    // A full dumpsys per package is slow; two dozen covers the apps people actually debug.
    const packages = await ipc.listCapturePackages(props.serial)
    report.value = await ipc.probeNetworkCapture(props.serial, packages.slice(0, 24))
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

/** Poll every live capture and merge by wall-clock key, so H5 and native traffic line up. */
async function pollAll(): Promise<void> {
  if (captures.value.length === 0 || fileMode.value) {
    stopPolling()
    return
  }
  const settled = await Promise.allSettled(
    captures.value.map((c) => ipc.pollNetworkCapture(c.id))
  )
  const next = { ...statuses.value }
  const merged: NetworkRequest[] = []
  let ok = 0
  for (const result of settled) {
    if (result.status !== 'fulfilled') continue
    ok += 1
    next[result.value.id] = result.value.status
    merged.push(...result.value.requests)
  }
  statuses.value = next
  merged.sort((a, b) =>
    (a.timeKey || '99:99:99.999').localeCompare(b.timeKey || '99:99:99.999')
  )
  requests.value = merged
  if (ok === 0 || Object.values(next).every((s) => !s.running)) stopPolling()
}

function beginLive(info: NetworkSessionInfo): void {
  if (fileMode.value || captures.value.length === 0) {
    requests.value = []
    selected.value = null
  }
  fileMode.value = false
  captures.value = [...captures.value, info]
  statuses.value = {
    ...statuses.value,
    [info.id]: {
      sessionId: info.id,
      running: true,
      entryCount: 0,
      events: 0,
      target: '',
      error: ''
    }
  }
  if (timer === null) timer = setInterval(() => void pollAll(), 800)
}

/**
 * Starting the first channel (or restarting after a stop) begins a fresh table. Starting
 * a second channel while one is live keeps both, which is the point of merging them.
 */
async function prepareLive(): Promise<void> {
  if (!running.value) {
    await closeAll()
    requests.value = []
    selected.value = null
  }
  fileMode.value = false
  boundSerial.value = props.serial
  // Capture is starting: give the table and the controls the room instead of the cards.
  showReport.value = false
}

async function startCdp(): Promise<void> {
  if (!canStartCdp.value) return
  busy.value = true
  error.value = ''
  try {
    await prepareLive()
    beginLive(await ipc.startCdpCapture({ serial: props.serial, socket: cdpSocket.value, port: 0 }))
  } catch (e) {
    error.value = errorMessage(e)
  } finally {
    busy.value = false
  }
}

async function startProxy(): Promise<void> {
  if (!canStartProxy.value) return
  busy.value = true
  error.value = ''
  try {
    await prepareLive()
    beginLive(await ipc.startProxyCapture({ serial: props.serial, port: 0 }))
  } catch (e) {
    error.value = errorMessage(e)
  } finally {
    busy.value = false
  }
}

async function stopAll(): Promise<void> {
  stopPolling()
  const failures: string[] = []
  for (const c of captures.value) {
    try {
      await ipc.stopNetworkCapture(c.id)
    } catch (e) {
      // The app-level cleanup still runs on window close, but the user must know the
      // device may still point at the now-closed proxy and can use the manual restore.
      failures.push(errorMessage(e))
    }
  }
  const next = { ...statuses.value }
  for (const id of Object.keys(next)) next[id] = { ...next[id], running: false }
  statuses.value = next
  if (failures.length > 0) {
    error.value = `停止采集时未能恢复设备代理：${failures.join('；')}。可在设备代理区域手动恢复。`
  }
}

async function closeAll(): Promise<void> {
  stopPolling()
  const failures: string[] = []
  for (const c of captures.value) {
    try {
      await ipc.closeNetworkCapture(c.id)
    } catch (e) {
      // Best effort; ProxyGuard::drop is the final safety net.
      failures.push(errorMessage(e))
    }
  }
  captures.value = []
  statuses.value = {}
  if (failures.length > 0) {
    error.value = `关闭采集时未能恢复设备代理：${failures.join('；')}。可在设备代理区域手动恢复。`
  }
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
    await closeAll()
    const info = await ipc.importNetworkCapture(picked)
    requests.value = await ipc.queryNetworkRequests(info.id)
    fileMode.value = true
    selected.value = null
  } catch (e) {
    error.value = errorMessage(e)
  }
}

async function configureProxy(): Promise<void> {
  error.value = ''
  try {
    proxyState.value = await ipc.configureDeviceProxy(props.serial, proxyInput.value)
    if (proxyState.value?.active) boundSerial.value = props.serial
  } catch (e) {
    error.value = errorMessage(e)
  }
}

async function restoreProxy(): Promise<void> {
  error.value = ''
  try {
    proxyState.value = await ipc.restoreDeviceProxy(restoreTarget.value)
    if (!proxyState.value?.active) boundSerial.value = ''
  } catch (e) {
    error.value = errorMessage(e)
  }
}

async function copyCurl(): Promise<void> {
  const text = curlText.value
  if (!text) return
  try {
    await navigator.clipboard.writeText(text)
    curlCopied.value = true
    if (curlTimer) clearTimeout(curlTimer)
    curlTimer = setTimeout(() => (curlCopied.value = false), 1600)
  } catch {
    error.value = '复制失败，请手动选中请求详情文本'
  }
}

/**
 * Called by the device dialog before it closes: stop both channels and give the device its
 * proxy back. Kept separate from `onBeforeUnmount` so the dialog can await it and show the
 * result, while a hard unmount still tries the same cleanup.
 */
async function shutdown(): Promise<void> {
  // Do not leave a CDP forward, a LAN listener or a global proxy behind a closed dialog.
  await closeAll()
  // Nothing was ever applied on this device: skip the restore entirely rather than run
  // `settings` writes against a device this tab never touched.
  const target = boundSerial.value || (proxyState.value?.active ? props.serial : '')
  boundSerial.value = ''
  if (!target) return
  try {
    proxyState.value = await ipc.restoreDeviceProxy(target)
  } catch {
    // ProxyGuard::drop is the final safety net.
  }
}

/**
 * Only probe while this tab is visible: the probe runs `dumpsys` on two dozen packages and
 * can take seconds, which would otherwise be paid the moment the dialog opens on the log tab.
 * Re-probing on device switch is required — a stale report would offer buttons for another
 * device's sockets.
 */
watch(
  () => [props.serial, props.active] as const,
  async ([serial, active], previous) => {
    if (!active) return
    if (!serial) {
      report.value = null
      return
    }
    const changed = previous?.[0] !== serial
    if (changed) {
      // Everything below belongs to the device we just left.
      report.value = null
      error.value = ''
      proxyState.value = null
    }
    if (report.value && !changed) return
    await runProbe()
  },
  { immediate: true }
)

onBeforeUnmount(() => {
  stopPolling()
  if (curlTimer) clearTimeout(curlTimer)
  // The component can be unmounted without going through shutdown() (window close,
  // store reset). Best effort only: an IPC may not get out while the webview is
  // being torn down. The Rust CloseRequested/Exit hooks and the proxy journal are
  // the real guarantee that the device is never left pointing at a dead port.
  const target = boundSerial.value || (proxyState.value?.active ? props.serial : '')
  void closeAll()
  if (target) void ipc.restoreDeviceProxy(target).catch(() => {})
})

defineExpose({ shutdown, isCapturing: running })

</script>

<template>
  <div class="device-tab-body">
    <div class="net-toolbar">
      <!--
        The dock is short: keep the environment verdict on one line and put the four-route
        reasoning behind a toggle. A wall of cards would push the request table — the thing
        people actually watch — below the fold.
      -->
      <template v-if="report">
        <span class="probe-badge" :class="report.recommended === 'cdp' ? 'ok' : 'muted'">
          推荐 {{ report.recommended }}
        </span>
        <span class="muted mono">
          Android {{ report.evidence.androidRelease }} / SDK {{ report.evidence.sdk }} ·
          {{ report.evidence.buildType }} ·
          {{ report.evidence.rootShell ? '有 root' : '无 root' }}
        </span>
      </template>
      <span v-else class="muted">
        体检会读取构建类型、WebView 调试通道与目标包的
        <code class="mono">network_security_config</code>，据此决定哪条采集路线可用。
      </span>
      <div class="spacer" />
      <button v-if="report" class="btn btn-ghost btn-sm" @click="showReport = !showReport">
        {{ showReport ? '收起详情' : '路线详情' }}
      </button>
      <button class="btn btn-sm" :disabled="!canProbe || busy" @click="runProbe">
        {{ probing ? '体检中…' : report ? '重新体检' : '运行环境体检' }}
      </button>
    </div>

    <p v-if="!serial" class="empty-hint warn">
      先在设备选择器里选一台已授权的设备，再回来体检并开始采集。
    </p>

    <template v-else>

      <!-- 代理自愈是异常状态，任何时候都要露出来，不能藏在折叠里 -->
      <p v-if="report?.healedProxy" class="probe-healed">
        已自动修复设备代理：{{ report.healedProxy }}
      </p>

      <template v-if="showReport && report">
        <div class="probe-summary">
          <p>{{ report.summary }}</p>
        </div>

        <div class="route-list">
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
      </template>

      <!-- 采集控制：WebView 与原生可同时开，请求合并到同一张表 -->
      <div class="net-controls">
        <button class="btn btn-primary btn-sm" :disabled="!canStartCdp" @click="startCdp">
          {{ busy ? '连接中…' : '开始 H5 采集' }}
        </button>
        <button class="btn btn-sm" :disabled="!canStartProxy" @click="startProxy">
          开始原生接口采集
        </button>
        <button class="btn btn-sm" :disabled="!running" @click="stopAll">停止采集</button>
        <button class="btn btn-sm" @click="pickCaptureFile">导入 HAR / JSONL</button>
        <div class="spacer" />
        <span v-if="running" class="live-hint">
          <span class="live-dot" />采集中 · {{ events }} 个事件
        </span>
        <span class="muted">{{ requests.length }} 条请求</span>
      </div>

      <!-- 解释性长文放进折叠区：它在第一次用时有用，之后只是占地方 -->
      <p v-if="showReport" class="net-native-note">
        <b>原生接口</b>走内建代理：明文 HTTP 可看到完整请求 / 响应；HTTPS 只能看到
        CONNECT 域名、TLS SNI、耗时与字节数。原生 HTTPS 明文需要 App 信任 MITM 证书，
        本机目标包未声明 <code class="mono">debug-overrides</code>，因此不做解密，也不会伪装成能看。
        采集结束后自动还原设备代理。
      </p>

      <p v-if="report && report.evidence.devtools.length && !report.evidence.devtools[0].pages" class="empty-hint warn">
        调试通道已就绪，但当前没有打开的 H5 页面。请在 App 里进入一个 WebView 页面，请求会自动出现在这里。
      </p>
      <p v-if="targets" class="empty-hint mono">采集端点：{{ targets }}</p>
      <p v-for="c in captures" :key="c.id" class="empty-hint mono">
        {{ c.label }} · {{ statuses[c.id]?.error || (statuses[c.id]?.running ? '运行中' : '已停止') }}
      </p>
      <p v-if="error" class="empty-hint error-text">{{ error }}</p>

      <!-- 请求列表：H5 + 原生按时间合并 -->
      <div class="net-table-wrap">
        <table class="net-table">
          <thead>
            <tr>
              <th style="width: 96px">时间</th>
              <th style="width: 48px">来源</th>
              <th style="width: 62px">方法</th>
              <th>URL</th>
              <th style="width: 190px">状态</th>
              <th style="width: 76px">耗时</th>
              <th style="width: 80px">大小</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="row in requests"
              :key="row.source + '-' + row.index"
              :class="{ picked: selected?.index === row.index && selected?.source === row.source }"
              @click="selected = row"
            >
              <td class="mono">{{ row.timeKey || row.startedAt || '—' }}</td>
              <td><span class="net-source" :class="row.source">{{ sourceLabel(row) }}</span></td>
              <td class="mono">{{ row.method }}</td>
              <td class="mono net-url" :title="row.url">{{ row.url }}</td>
              <td class="mono" :class="statusClass(row)">{{ statusText(row) }}</td>
              <td class="mono">{{ row.durationMs }} ms</td>
              <td class="mono">{{ formatSize(row.responseSize) }}</td>
            </tr>
            <tr v-if="requests.length === 0">
              <td colspan="7" class="net-empty">
                {{ running ? '等待设备请求…（在 App 里触发接口调用或打开 H5 页面）' : '暂无请求。可以开始 H5 / 原生接口采集，或导入一份 HAR / JSONL 抓包文件。' }}
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
          <button
            v-if="curlText"
            class="btn btn-ghost btn-sm"
            :class="{ copied: curlCopied }"
            :title="
              selected.redacted
                ? '复制为 cURL 命令（token / Cookie 等为脱敏值）'
                : '复制为 cURL 命令'
            "
            @click="copyCurl"
          >
            {{ curlCopied ? '已复制' : '复制 cURL' }}
          </button>
          <button
            class="btn btn-ghost btn-sm"
            title="关闭请求详情"
            aria-label="关闭请求详情"
            @click="selected = null"
          >×</button>
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

      <!-- 手动代理：供外部 MITM 工具使用，与内建代理采集互斥 -->
      <div class="net-advanced">
        <button class="btn btn-ghost btn-sm" @click="showAdvanced = !showAdvanced">
          {{ showAdvanced ? '收起' : '展开' }} · 手动 MITM 代理设置（外部抓包工具）
        </button>
        <div v-if="showAdvanced" class="net-advanced-body">
          <div class="form-row">
            <span class="form-label">代理</span>
            <input v-model="proxyInput" class="select mono" style="flex: 1" placeholder="例如 10.0.0.2:8080" />
            <button class="btn btn-sm" :disabled="!serial || !proxyInput" @click="configureProxy">设置</button>
            <button class="btn btn-sm" :disabled="!proxyState?.active" @click="restoreProxy">还原</button>
          </div>
          <p class="empty-hint">
            这里是给 Charles / mitmproxy 等外部工具手动指向设备用的；上面「原生接口采集」会自动设置并还原代理，两者不要同时用。
            Android 7+ 的 release 包不信任用户 CA，HTTPS 明文仍取决于证书条件。
          </p>
          <p v-if="proxyState?.active" class="empty-hint warn mono">
            当前生效：{{ proxyState.proxy }}（原值：{{ proxyState.original || '无' }}）
          </p>
        </div>
      </div>
    </template>

    <p class="empty-hint net-foot-note">
      请求与响应中的 token / Cookie 在落地时即脱敏，不会进入导出文件。
      关闭本面板会停止采集并还原设备代理。
    </p>
  </div>
</template>
