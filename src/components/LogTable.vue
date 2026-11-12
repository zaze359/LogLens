<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { loadMore, store } from '../stores/session'
import type { LogEntry } from '../types'

const ROW_H = 26
const viewport = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewportHeight = ref(600)
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
  if (el.scrollHeight - el.scrollTop - el.clientHeight < 320) void loadMore()
}

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
    <span>Tag</span>
    <span>消息（点击行查看完整内容与堆栈）</span>
  </div>

  <div ref="viewport" class="table-body" @scroll="onScroll">
    <div v-if="store.entries.length === 0" class="table-empty">
      <template v-if="store.loading">正在解析并筛选日志…</template>
      <template v-else-if="!store.fileInfo">尚未打开日志文件。点击右上角「打开日志文件」，或直接把文件拖进窗口。</template>
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
  </div>
</template>
