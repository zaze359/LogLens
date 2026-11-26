<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { currentHitIndex, loadMore, store } from '../stores/session'
import type { LogEntry } from '../types'

const ROW_H = 26
/** Cell padding plus a little breathing room, so the last glyph never touches the edge. */
const MSG_PADDING = 26
/** Room for the "+N 行" chip on entries that collapsed a stack trace. */
const CONT_BADGE = 70
/** Sanity cap: one 50k-char JSON payload must not make the table 400 000px wide. */
const MAX_MSG_PX = 20000

const viewport = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
/** Horizontal offset, mirrored onto the header so the labels stay above their columns. */
const scrollLeft = ref(0)
const viewportHeight = ref(600)
/** Live tail: keep the newest rows in view unless the user scrolled up on purpose. */
const follow = ref(true)
let observer: ResizeObserver | null = null
let resizeRaf = 0

const startIndex = computed(() => Math.max(0, Math.floor(scrollTop.value / ROW_H) - 6))
const endIndex = computed(() =>
  Math.min(store.entries.length, startIndex.value + Math.ceil(viewportHeight.value / ROW_H) + 14)
)
const visible = computed(() => store.entries.slice(startIndex.value, endIndex.value))
const offsetY = computed(() => startIndex.value * ROW_H)
const spacerHeight = computed(() => Math.max(store.entries.length * ROW_H, viewportHeight.value))

/* ------------------------------------------------------- message column ---- */

let measureCtx: CanvasRenderingContext2D | null = null

/**
 * Real glyph width for the table font. `1ch` is only correct for characters that render at
 * the monospace advance, and log payloads mix CJK, emoji and ASCII; measuring the actual
 * string avoids a column that is too narrow (still ellipsising) or absurdly wide.
 */
function measure(text: string): number {
  if (!measureCtx) {
    measureCtx = document.createElement('canvas').getContext('2d')
    if (measureCtx) {
      measureCtx.font =
        '12px ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace'
    }
  }
  if (!measureCtx) return text.length * 7.2
  return measureCtx.measureText(text).width
}

/**
 * Width of the message column: the widest **first line** among the loaded entries.
 *
 * Rows are single-line by design (fixed 26px height is what makes virtual scrolling cheap),
 * so the only way to show a long line in full is to let the table scroll sideways. Sizing the
 * column from the data — instead of a per-row `max-content` — keeps the width stable while
 * scrolling, since the browser would otherwise recompute it from whichever rows happen to be
 * rendered and the horizontal scrollbar would jump on every scroll.
 */
const msgWidth = computed(() => {
  let worst = 0
  for (const entry of store.entries) {
    const nl = entry.message.indexOf('\n')
    const line = nl < 0 ? entry.message : entry.message.slice(0, nl)
    const width = measure(line) + (entry.continuation > 0 ? CONT_BADGE : 0)
    if (width > worst) worst = width
  }
  const needed = Math.ceil(worst) + MSG_PADDING
  // Never narrower than what is left of the viewport, so short logs still fill the width.
  return Math.min(Math.max(needed, 240), MAX_MSG_PX)
})

const tableStyle = computed(
  () => ({ '--msg-w': `${msgWidth.value}px` }) as Record<string, string>
)

onMounted(() => {
  const el = viewport.value
  if (!el) return
  viewportHeight.value = el.clientHeight || 600
  observer = new ResizeObserver(() => {
    // Writing viewportHeight synchronously here changes the virtual row count and spacer
    // height, which re-lays out this element and re-fires the observer in the same frame —
    // the browser then reports "ResizeObserver loop completed with undelivered notifications".
    // Coalesce to one read+write per animation frame (and skip no-op writes) to break it.
    if (resizeRaf) return
    resizeRaf = requestAnimationFrame(() => {
      resizeRaf = 0
      const next = el.clientHeight || 600
      if (next !== viewportHeight.value) viewportHeight.value = next
    })
  })
  observer.observe(el)
})

onBeforeUnmount(() => {
  if (resizeRaf) cancelAnimationFrame(resizeRaf)
  observer?.disconnect()
})

function onScroll(e: Event): void {
  const el = e.target as HTMLElement
  scrollTop.value = el.scrollTop
  // The header is a sibling of the scroll container, so it has to be translated by hand to
  // stay above its columns while the table is scrolled sideways.
  scrollLeft.value = el.scrollLeft
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
    // A narrower tab clamps the offset without emitting a scroll event; resync so the
    // header does not stay shifted by the previous tab's horizontal position.
    scrollLeft.value = el.scrollLeft
    follow.value = max - top < 48
  }
)

// A "find" jump asks the table to bring one row into view. `seq` makes repeated jumps to the
// same row work, and the row may have been paged in only just now — hence `nextTick`.
watch(
  () => store.scrollTarget,
  async (target) => {
    if (!target) return
    await nextTick()
    const el = viewport.value
    if (!el) return
    const top = Math.max(0, target.index * ROW_H - Math.round(viewportHeight.value / 3))
    el.scrollTop = Math.min(top, el.scrollHeight)
    scrollTop.value = el.scrollTop
    store.scrollTop = el.scrollTop
    follow.value = false
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
  <div class="table-head" :style="tableStyle">
    <div class="table-head-inner" :style="{ transform: `translateX(${-scrollLeft}px)` }">
      <span style="text-align:right">#</span>
      <span>级别</span>
      <span>时间</span>
      <span>PID</span>
      <span>TID</span>
      <span>包名</span>
      <span>Tag</span>
      <span>消息 · 左右滚动看全文</span>
    </div>
  </div>

  <div class="table-viewport">
  <div ref="viewport" class="table-body" :style="tableStyle" @scroll="onScroll">
    <div v-if="store.entries.length === 0" class="table-empty">
      <template v-if="store.loading">正在解析并筛选日志…</template>
      <template v-else-if="!store.fileInfo">
        尚未打开日志文件。点击右上角「打开日志文件」，或直接把文件拖进窗口。
      </template>
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
          :class="[
            levelClass(entry.level),
            {
              selected: store.selected?.index === entry.index,
              hit: currentHitIndex === entry.index,
              unparsed: !entry.parseOk
            }
          ]"
          @click="store.selected = entry"
        >
          <span class="cell cell-index">{{ entry.index + 1 }}</span>
          <span class="badge-level">{{ entry.level }}</span>
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
  </div>

  <!-- Outside the scrolling `.table-body`: a sticky overlay inside the same scroller gets
       painted under `.table-rows`' compositor layer in WKWebView, so the pill was intermittently
       hidden behind log lines. Absolute within the non-scrolling wrapper avoids that entirely. -->
  <div v-if="store.deviceStream && !follow" class="follow-jump">
    <button class="btn btn-sm" @click="jumpToLatest">↓ 回到最新</button>
  </div>
  </div>
</template>
