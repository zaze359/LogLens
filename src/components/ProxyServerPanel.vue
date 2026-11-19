<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import {
  closeProxyPanel,
  refreshProxyServer,
  startProxyServer,
  stopProxyServer,
  store
} from '../stores/session'
import { errorMessage, ipc } from '../lib/ipc'
import type { NetworkRequest } from '../types'

/**
 * 独立代理：本机开一个普通 HTTP 代理，不绑定任何设备、不碰 adb。
 *
 * 与设备里的「开始原生接口采集」分工：那边是把「开代理 + 给这台设备设代理」打包成一步
 * 自动化；这里只负责开代理，任何客户端（另一台手机、模拟器、已手动配好代理的 App）
 * 只要把代理指向这里显示的地址，请求就会出现在下面，按来源 IP 分组。
 */
/**
 * 状态不放在组件里：顶栏要常驻显示，面板关掉后也要继续跟踪，所以唯一事实来源是
 * `store.proxyServer`，这里只读它。
 */
const status = computed(() => store.proxyServer)
/** 后端每次 poll 只返回新增的行，追加时要一个稳定的 key：后端没有全局序号。 */
let seq = 0
type Row = NetworkRequest & { key: number }
const rows = ref<Row[]>([])
const busy = ref(false)
const error = ref('')
const copied = ref(false)

const live = computed(() => !!status.value?.running)
const endpoint = computed(() => status.value?.endpoint ?? '')

/** 按来源地址分组：同一台客户端的所有请求归到一组，组内按时间先后。 */
const groups = computed(() => {
  const byDevice = new Map<string, Row[]>()
  for (const row of rows.value) {
    const key = row.device || '未知来源'
    const list = byDevice.get(key) ?? []
    list.push(row)
    byDevice.set(key, list)
  }
  return [...byDevice.entries()].map(([device, list]) => ({
    device,
    rows: list,
    last: list[list.length - 1]?.timeKey ?? ''
  }))
})

let timer: ReturnType<typeof setInterval> | null = null
let polling = false

function stopPolling(): void {
  if (timer !== null) {
    clearInterval(timer)
    timer = null
  }
}

async function poll(): Promise<void> {
  if (polling || !live.value) return
  polling = true
  try {
    const fresh = await ipc.pollProxyServer()
    if (fresh.length > 0) {
      rows.value = [...rows.value, ...fresh.map((row) => ({ ...row, key: ++seq }))]
      // 只保留最近 500 条：面板是临时取证用的，无限增长会拖慢整个窗口。
      if (rows.value.length > 500) rows.value = rows.value.slice(-500)
    }
    error.value = ''
  } catch (e) {
    // 采集还在跑时不因一次读取失败就停：与抓包表格同一套策略。
    error.value = `读取代理请求失败，正在重试：${errorMessage(e)}`
  } finally {
    polling = false
  }
}

function ensurePolling(): void {
  if (timer === null && live.value) timer = setInterval(() => void poll(), 800)
}

async function toggle(): Promise<void> {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    if (live.value) {
      await stopProxyServer()
      stopPolling()
    } else {
      // 0 = 默认端口（18999）；被占用时后端会退到系统分配的端口，并通过 endpoint 回报。
      await startProxyServer()
      rows.value = []
      ensurePolling()
    }
  } catch (e) {
    error.value = errorMessage(e)
  } finally {
    busy.value = false
  }
}

async function copyEndpoint(): Promise<void> {
  if (!endpoint.value) return
  try {
    await navigator.clipboard.writeText(endpoint.value)
    copied.value = true
    setTimeout(() => (copied.value = false), 1600)
  } catch {
    error.value = '复制失败，请手动选中地址复制'
  }
}

function clearRows(): void {
  rows.value = []
}

onMounted(async () => {
  // 面板是"查看"入口：状态store 里已有就先用，没有或过期才问一次后端。
  await refreshProxyServer()
  ensurePolling()
})

onBeforeUnmount(() => {
  // 面板关掉不等于代理关掉：用户可能正靠它抓别的设备。轮询停在这里，代理保持开启。
  stopPolling()
})
</script>

<template>
  <div class="modal-mask">
    <div class="modal proxy-modal">
      <div class="modal-head">
        <span>独立代理</span>
        <span class="badge" :class="{ 'proxy-on': live }">{{ live ? '运行中' : '未开启' }}</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" title="关闭面板（不停止代理）" @click="closeProxyPanel">
          ×
        </button>
      </div>

      <div class="proxy-body">
        <p class="empty-hint">
          全应用只有一个抓包代理。把任意设备的代理指向下面的地址即可抓包；这里只显示
          <b>没有绑定设备</b>的请求，按来源 IP 分组。设备工作区里的「开始原生接口采集」
          只是自动替那台设备设好代理，它的请求显示在设备页，不会出现在这里。
        </p>

        <div class="form-row">
          <span class="form-label">代理地址</span>
          <input
            class="select mono"
            style="flex: 1"
            :value="live ? endpoint : '开启后显示'"
            readonly
          />
          <button class="btn btn-sm" :disabled="!live" @click="copyEndpoint">
            {{ copied ? '已复制' : '复制' }}
          </button>
          <button
            class="btn btn-sm"
            :class="live ? 'btn-danger-ghost' : 'btn-primary'"
            :disabled="busy"
            @click="toggle"
          >
            {{ busy ? '处理中…' : live ? '停止代理' : '开启代理' }}
          </button>
        </div>

        <p v-if="live" class="empty-hint mono">
          已在 {{ status?.lan }}:{{ status?.port }} 监听 · 未绑定来源 {{ status?.sources ?? 0 }} 个
          · 已绑定设备 {{ status?.devices ?? 0 }} 台 · 共收到 {{ rows.length }} 条请求
        </p>
        <p v-if="error" class="empty-hint error-text">{{ error }}</p>

        <div class="proxy-groups">
          <section v-for="group in groups" :key="group.device" class="proxy-group">
            <div class="proxy-group-head">
              <b class="mono">{{ group.device }}</b>
              <span class="muted">{{ group.rows.length }} 条</span>
              <div class="spacer" />
              <span class="muted mono">{{ group.last }}</span>
            </div>
            <table class="net-table">
              <tbody>
                <tr v-for="row in group.rows" :key="row.key">
                  <td class="mono" style="width: 96px">{{ row.timeKey || '—' }}</td>
                  <td class="mono" style="width: 62px">{{ row.method }}</td>
                  <td class="mono net-url" :title="row.url">{{ row.url }}</td>
                  <td class="mono" style="width: 60px">{{ row.status || row.error || '—' }}</td>
                  <td class="mono" style="width: 70px">{{ row.durationMs }} ms</td>
                </tr>
              </tbody>
            </table>
          </section>

          <p v-if="groups.length === 0" class="empty-hint">
            {{ live ? '等待客户端连接…（把设备代理指向上面的地址后，请求会自动出现在这里）' : '代理尚未开启。' }}
          </p>
        </div>
      </div>

      <div class="modal-foot">
        <button class="btn btn-ghost btn-sm" :disabled="rows.length === 0" @click="clearRows">
          清空列表
        </button>
        <span class="empty-hint" style="margin-right: auto">
          抓包内容按原文保存，含 token / Cookie 等凭据，请勿随意外发。
        </span>
      </div>
    </div>
  </div>
</template>
