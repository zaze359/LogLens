<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { activateTab, startDeviceStream, stopDeviceStream, store } from '../../stores/session'
import type { SessionTab } from '../../stores/session'

/**
 * 日志工具的采集状态条 —— 抓取是这台设备的一件事，所以「是不是在抓、抓了多少、抓多久、
 * 怎么停」都摆在这里，而不是跑到顶栏去变成一个离开设备也还在的按钮。
 *
 * 数据全部来自 store：`entryCount` 由后台轮询更新，速率是这里的 1 秒采样差分。
 */
const props = defineProps<{ serial: string }>()

/** 这台设备的日志会话（可能不止一份：停过一次再开始就多一份）。 */
const logs = computed<SessionTab[]>(() =>
  store.tabs.filter((tab) => tab.info.device === props.serial && tab.info.source === 'stream')
)

const tab = computed<SessionTab | null>(
  () => logs.value.find((candidate) => candidate.id === store.activeId) ?? logs.value[0] ?? null
)

const running = computed<boolean>(() => !!tab.value?.stream?.running)
const count = computed<number>(() => tab.value?.info.entryCount ?? 0)

const rate = ref(0)
const elapsed = ref('')
const busy = ref(false)

let timer: ReturnType<typeof setInterval> | null = null
let lastCount = 0
let lastAt = 0

/** `mm:ss`，超过一小时才补上小时位，免得常态下多两个 0。 */
function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  const mm = String(m).padStart(2, '0')
  const ss = String(s).padStart(2, '0')
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`
}

function tick(): void {
  const now = Date.now()
  if (running.value) {
    const startedAt = tab.value?.stream?.startedAt ?? now
    elapsed.value = formatElapsed(now - startedAt)
    if (lastAt > 0) {
      const seconds = (now - lastAt) / 1000
      // 轮询间隔约 0.8s，采样窗口 1s：四舍五入到整数条/秒足够，不需要平滑。
      rate.value = seconds > 0 ? Math.max(0, Math.round((count.value - lastCount) / seconds)) : 0
    }
  } else {
    elapsed.value = ''
    rate.value = 0
  }
  lastCount = count.value
  lastAt = now
}

onMounted(() => {
  tick()
  timer = setInterval(tick, 1000)
})
onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})

const buffersText = computed<string>(() => {
  const buffers = store.lastCapture?.buffers ?? []
  return buffers.length ? buffers.join(' + ') : '默认（main + system + crash）'
})

/** 停过一次再开始会新建一份日志；多份时给个入口，否则旧的只能靠关掉当前的才看得到。 */
const otherLogs = computed<SessionTab[]>(() => logs.value.filter((item) => item.id !== tab.value?.id))

function switchLog(id: string): void {
  activateTab(id)
}

async function stop(): Promise<void> {
  if (busy.value) return
  busy.value = true
  try {
    await stopDeviceStream()
  } finally {
    busy.value = false
  }
}

/** 同一份日志停掉后，用上次的选项重新开始（会新建一份，旧的保留为静态日志）。 */
async function restart(): Promise<void> {
  if (busy.value) return
  busy.value = true
  try {
    await startDeviceStream(
      store.lastCapture ?? { serial: props.serial, buffers: [], clearBefore: false }
    )
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="capture-bar" :class="{ live: running }">
    <span class="capture-dot" :class="{ live: running }" />
    <span class="capture-state">{{ running ? '实时抓取中' : '已停止' }}</span>

    <span class="capture-stat">{{ count.toLocaleString() }} 条</span>
    <template v-if="running">
      <span class="capture-stat" title="最近 1 秒新增条数">{{ rate }} 条/秒</span>
      <span class="capture-stat mono">{{ elapsed }}</span>
    </template>
    <span class="capture-stat muted">缓冲区 {{ buffersText }}</span>

    <span v-if="otherLogs.length" class="capture-logs">
      <span class="muted">其他日志</span>
      <button
        v-for="item in otherLogs"
        :key="item.id"
        class="capture-log-link"
        :title="`切换到 ${item.title}（${item.info.entryCount} 条 · 已停止）`"
        @click="switchLog(item.id)"
      >
        {{ item.info.entryCount.toLocaleString() }} 条
      </button>
    </span>

    <div class="spacer" />
    <button
      v-if="running"
      class="btn btn-sm btn-danger-ghost"
      :disabled="busy"
      title="停止 adb logcat（已收到的日志保留，仍可筛选与导出）"
      @click="stop"
    >
      {{ busy ? '停止中…' : '停止抓取' }}
    </button>
    <button
      v-else
      class="btn btn-sm"
      :disabled="busy"
      title="用上次的缓冲区设置重新开始（同一台设备只会有一个实时会话）"
      @click="restart"
    >
      {{ busy ? '启动中…' : '重新开始' }}
    </button>
  </div>
</template>
