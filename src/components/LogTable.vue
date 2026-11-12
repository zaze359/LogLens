<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { loadMore, store } from '../stores/session'
import type { LogEntry } from '../types'

const ROW_H = 26
const viewport = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewportHeight = ref(600)
/** Live tail: keep the newest rows in view unless the user scrolled up on purpose. */
const follow = ref(true)
let observer: ResizeObserver | null = null

const startIndex = computed(() => Math.max(0, Math.floor(scrollTop.value / ROW_H) - 6))
const endIndex = computed(() =>
  Math.min(store.entries.length, startIndex.value + Math.ceil(viewportHeight.value / ROW_H) + 14)
)
const visible = computed(() => store.entries.slice(startIndex.value, endIndex.value))
const offsetY = computed(() => startIndex.value * ROW_H)
const spacerHeight = computed(() => Math.max(store.entries.length * ROW_H, viewportHeight.value))

onMounted(() => {
  const el = viewport.value
  if (!el) return
  viewportHeight.value = el.clientHeight || 600
  observer = new ResizeObserver(() => {
    viewportHeight.value = el.clientHeight || 600
  })
  observer.observe(el)
})

onBeforeUnmount(() => observer?.disconnect())

function onScroll(e: Event): void {
  const el = e.target as HTMLElement
  scrollTop.value = el.scrollTop
  // Remembered per tab, so switching away and back lands on the same line.
  store.scrollTop = el.scrollTop
  const distance = el.scrollHeight - el.scrollTop - el.clientHeight
  // Scrolling up opts out of auto-follow until the user comes back to the bottom.
  follow.value = distance < 48
  if (distance < 320) void loadMore()
}

function jumpToLatest(): void {
  follow.value = true
  const el = viewport.value
  if (el) el.scrollTop = el.scrollHeight
}

// New entries always append at the end, so a live capture looks frozen when the viewport
// sits at the top. Follow the tail while streaming, and stop as soon as the user scrolls up.
watch(
  () => store.entries.length,
  async () => {
    if (!store.deviceStream || !follow.value) return
    await nextTick()
    const el = viewport.value
    if (el) el.scrollTop = el.scrollHeight
  }
)

// Each tab remembers where you were. `activeId` changes after the new entries are in place,
// so read the offset here rather than trying to restore it from the tab strip.
watch(
  () => store.activeId,
  async () => {
    await nextTick()
    const el = viewport.value
    if (!el) return
    const max = Math.max(0, el.scrollHeight - el.clientHeight)
    const top = Math.min(store.scrollTop, max)
    el.scrollTop = top
    scrollTop.value = top
    follow.value = max - top < 48
  }
)

function levelClass(level: string): string {
  return `lv-${level === '?' ? 'unknown' : level.toLowerCase()}`
}

interface Segment {
  text: string
  hit: boolean
}

function segments(entry: LogEntry): Segment[] {
  const line = entry.message.split('\n')[0] ?? ''
  const term = store.filter.search
  if (!term.trim() || store.filter.isRegex) return [{ text: line, hit: false }]
  const hay = store.filter.caseSensitive ? line : line.toLowerCase()
  const needle = store.filter.caseSensitive ? term : term.toLowerCase()
  const out: Segment[] = []
  let pos = 0
  for (;;) {
    const idx = hay.indexOf(needle, pos)
    if (idx < 0) {
      out.push({ text: line.slice(pos), hit: false })
      break
    }
    if (idx > pos) out.push({ text: line.slice(pos, idx), hit: false })
    out.push({ text: line.slice(idx, idx + needle.length), hit: true })
    pos = idx + needle.length
  }
  return out
}
</script>

<template>
  <div class="table-head">
    <span style="text-align:right">#</span>
    <span>级别</span>
    <span>时间</span>
    <span>PID</span>
    <span>TID</span>
    <span>包名</span>
    <span>Tag</span>
    <span>消息（点击行查看完整内容与堆栈）</span>
  </div>

  <div ref="viewport" class="table-body" @scroll="onScroll">
    <div v-if="store.entries.length === 0" class="table-empty">
      <template v-if="store.loading">正在解析并筛选日志…</template>
      <template v-else-if="!store.fileInfo">尚未打开日志文件。点击右上角「打开日志文件」，或直接把文件拖进窗口。</template>
      <template v-else-if="store.deviceStream">
        实时抓取中，正在等待设备产生日志。在手机上操作一下，新日志会自动出现在这里。
      </template>
      <template v-else>当前筛选条件下没有匹配的日志。试试清空筛选或换一个模板。</template>
    </div>

    <div v-else class="table-spacer" :style="{ height: spacerHeight + 'px' }">
      <div class="table-rows" :style="{ transform: `translateY(${offsetY}px)` }">
        <div
          v-for="entry in visible"
          :key="entry.index"
          class="log-row"
          :class="{ selected: store.selected?.index === entry.index, unparsed: !entry.parseOk }"
          @click="store.selected = entry"
        >
          <span class="cell cell-index">{{ entry.index + 1 }}</span>
          <span class="badge-level" :class="levelClass(entry.level)">{{ entry.level }}</span>
          <span class="cell cell-time">{{ entry.timestamp || '—' }}</span>
          <span class="cell cell-pid">{{ entry.pid || '—' }}</span>
          <span class="cell cell-pid">{{ entry.tid || '—' }}</span>
          <span class="cell cell-pkg" :title="entry.package">{{ entry.package || '—' }}</span>
          <span class="cell cell-tag" :title="entry.tag">{{ entry.tag || '—' }}</span>
          <span class="cell cell-msg">
            <template v-for="(seg, i) in segments(entry)" :key="i">
              <mark v-if="seg.hit">{{ seg.text }}</mark>
              <template v-else>{{ seg.text }}</template>
            </template>
            <span v-if="entry.continuation > 0" class="cont">+{{ entry.continuation }} 行</span>
          </span>
        </div>
      </div>
    </div>

    <div v-if="store.deviceStream && !follow" class="follow-jump">
      <button class="btn btn-sm" @click="jumpToLatest">↓ 回到最新</button>
    </div>
  </div>
</template>
