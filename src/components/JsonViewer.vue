<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { flattenJson, parseJsonBody } from '../lib/json'
import type { JsonLine } from '../lib/json'

/**
 * 通用 JSON 查看器：正文来自抓包时直接进入格式化树视图，空白打开时停在原文方便粘贴；
 * 「原文 / 格式化」可随时切换，支持逐节点折叠与复制。不是合法 JSON 时格式化按钮不可用，
 * 始终按原文展示、不改动内容。
 */
const props = defineProps<{ text: string; label: string }>()

type ViewMode = 'raw' | 'pretty'

/** 带正文进来（抓包详情）就直接看格式化结果；否则留在原文，等待用户粘贴。 */
function preferredMode(text: string): ViewMode {
  return text.trim() && parseJsonBody(text).ok ? 'pretty' : 'raw'
}

const mode = ref<ViewMode>(preferredMode(props.text))
/** 本地可编辑的正文。面板里允许用户粘贴任意 JSON，不能改动 prop。 */
const source = ref(props.text)
/** 折叠状态的容器 path 集合；换一条正文时清空，避免沿用上一次的折叠位置。 */
const collapsed = ref<Set<string>>(new Set())
const copied = ref(false)
let copyTimer: ReturnType<typeof setTimeout> | null = null

const parsed = computed(() => parseJsonBody(source.value))
const isJson = computed(() => parsed.value.ok && !!source.value.trim())
/** 格式化视图要展示的行：祖先被折叠的整段隐藏，折叠容器自身的闭合行也隐藏。 */
const lines = computed(() => {
  if (!isJson.value || parsed.value.value === null) return []
  const folded = collapsed.value
  return flattenJson(parsed.value.value).filter(
    (line) =>
      !line.ancestors.some((path) => folded.has(path)) &&
      !(line.kind.startsWith('close') && folded.has(line.path))
  )
})
const allContainerPaths = computed(() =>
  flattenJson(parsed.value.value ?? null).filter(isOpen).map((line) => line.path)
)
/** 复制按钮取的文本：格式化视图给规范缩进的 JSON，原文视图给正在编辑的原样字符串。 */
const copyText = computed(() =>
  mode.value === 'pretty' && isJson.value ? parsed.value.formatted : source.value
)

function isOpen(line: JsonLine): boolean {
  return line.kind === 'open-object' || line.kind === 'open-array'
}

function isCollapsed(path: string): boolean {
  return collapsed.value.has(path)
}

/** 标量行的逗号跟在自己行尾；容器的逗号跟闭合行，折叠时改挂在开口行上。 */
function showComma(line: JsonLine): boolean {
  if (!line.comma) return false
  return (
    line.kind === 'value' ||
    line.kind.startsWith('close') ||
    (isOpen(line) && isCollapsed(line.path))
  )
}

function toggle(path: string): void {
  const next = new Set(collapsed.value)
  if (next.has(path)) next.delete(path)
  else next.add(path)
  collapsed.value = next
}

function expandAll(): void {
  collapsed.value = new Set()
}

function collapseAll(): void {
  collapsed.value = new Set(allContainerPaths.value)
}

function setMode(next: ViewMode): void {
  if (next === 'pretty' && !isJson.value) return
  mode.value = next
}

/* ---------- 关键字搜索：高亮所有命中，并把当前命中滚动到视野中间 ---------- */

/** 一段文本要么是普通片段（hit = -1），要么是第 N 个命中。 */
interface Segment {
  text: string
  hit: number
}
interface Hit {
  /** 格式化视图里命中所在的行号，用于滚动定位。 */
  line: number
  /** 原文视图里命中在整段文本中的字符偏移，用于滚动与选中。 */
  start: number
}
interface SearchRow {
  key: Segment[]
  value: Segment[]
}

const query = ref('')
/** 当前命中的序号；命中的集合会随折叠 / 编辑 / 切换视图变化，比较时统一走 activeHit。 */
const matchIndex = ref(0)
const tree = ref<HTMLElement | null>(null)
const rawEditor = ref<HTMLTextAreaElement | null>(null)
const rawLayer = ref<HTMLElement | null>(null)

/** 格式化视图：逐行切段，key 与 value 都参与匹配。 */
const prettySearch = computed<{ rows: SearchRow[]; hits: Hit[] }>(() => {
  const needle = query.value.trim().toLowerCase()
  const hits: Hit[] = []
  const rows = lines.value.map((line, index) => {
    if (!needle) {
      return {
        key: line.key === null ? [] : [{ text: line.key, hit: -1 }],
        value: [{ text: line.text, hit: -1 }]
      }
    }
    return {
      key: line.key === null ? [] : splitHits(line.key, needle, index, hits),
      value: splitHits(line.text, needle, index, hits)
    }
  })
  return { rows, hits }
})

/** 原文视图：整段文本切段，段落按顺序拼接后必须与原文完全一致（含空白与换行）。 */
const rawSearch = computed<{ segments: Segment[]; hits: Hit[] }>(() => {
  const needle = query.value.trim().toLowerCase()
  const text = source.value
  if (!needle) return { segments: [{ text, hit: -1 }], hits: [] }
  const hits: Hit[] = []
  const lower = text.toLowerCase()
  const segments: Segment[] = []
  let from = 0
  for (let at = lower.indexOf(needle); at >= 0; at = lower.indexOf(needle, from)) {
    if (at > from) segments.push({ text: text.slice(from, at), hit: -1 })
    segments.push({ text: text.slice(at, at + needle.length), hit: hits.length })
    hits.push({ line: -1, start: at })
    from = at + needle.length
  }
  if (from < text.length) segments.push({ text: text.slice(from), hit: -1 })
  if (segments.length === 0) segments.push({ text, hit: -1 })
  return { segments, hits }
})

const rows = computed(() => prettySearch.value.rows)
const rawSegments = computed(() => rawSearch.value.segments)
/** 两种视图共用计数与 ↑/↓，切换视图后从新视图的第一个命中开始。 */
const hits = computed(() => (mode.value === 'raw' ? rawSearch.value.hits : prettySearch.value.hits))
/** 折叠 / 编辑后命中数可能变少，计数器永远落在一个真实存在的命中上。 */
const activeHit = computed(() =>
  hits.value.length === 0 ? -1 : Math.min(matchIndex.value, hits.value.length - 1)
)

/** 按小写匹配切段，命中段记录自己的全局序号，渲染时与 activeHit 对比即可。 */
function splitHits(text: string, needle: string, line: number, hits: Hit[]): Segment[] {
  if (!text) return [{ text: '', hit: -1 }]
  const lower = text.toLowerCase()
  const segments: Segment[] = []
  let from = 0
  for (let at = lower.indexOf(needle); at >= 0; at = lower.indexOf(needle, from)) {
    if (at > from) segments.push({ text: text.slice(from, at), hit: -1 })
    segments.push({ text: text.slice(at, at + needle.length), hit: hits.length })
    hits.push({ line, start: -1 })
    from = at + needle.length
  }
  if (from < text.length) segments.push({ text: text.slice(from), hit: -1 })
  return segments
}

/** 等宽字体下单字符宽度；只在切换正文 / 改字号时才重新测量。 */
let monoWidth: { key: string; width: number } | null = null
function monoCharWidth(el: HTMLElement): number {
  const style = getComputedStyle(el)
  const key = `${style.fontStyle}|${style.fontWeight}|${style.fontSize}|${style.fontFamily}`
  if (monoWidth?.key === key) return monoWidth.width
  const context = document.createElement('canvas').getContext('2d')
  if (context) context.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`
  const width = context?.measureText('0').width || parseFloat(style.fontSize) * 0.6
  monoWidth = { key, width }
  return width
}

/** 原文是 textarea，按行列算出偏移再滚过去；同时选中它，方便直接复制。 */
function scrollRawTo(start: number): void {
  const editor = rawEditor.value
  if (!editor) return
  const before = source.value.slice(0, start)
  const lineIndex = before.split('\n').length - 1
  // tab-size: 2，按两列估算列宽，否则带缩进的命中会横向偏一点。
  const column = before.slice(before.lastIndexOf('\n') + 1).replace(/\t/g, '  ').length
  const style = getComputedStyle(editor)
  const lineHeight = parseFloat(style.lineHeight) || 19
  const needleLength = query.value.trim().length
  editor.setSelectionRange(start, start + needleLength)
  editor.scrollTop = Math.max(0, lineIndex * lineHeight - editor.clientHeight / 2)
  editor.scrollLeft = Math.max(0, column * monoCharWidth(editor) - editor.clientWidth / 2)
  syncRawScroll()
}

/** 原文层与 textarea 是两个元素，滚动位置必须同步，否则高亮会错位。 */
function syncRawScroll(): void {
  const editor = rawEditor.value
  const layer = rawLayer.value
  if (!editor || !layer) return
  layer.scrollTop = editor.scrollTop
  layer.scrollLeft = editor.scrollLeft
}

function scrollToActive(): void {
  const hit = hits.value[activeHit.value]
  if (!hit) return
  if (mode.value === 'raw') {
    scrollRawTo(hit.start)
    return
  }
  const line = tree.value?.querySelector<HTMLElement>(`[data-line="${hit.line}"]`)
  line?.scrollIntoView({ block: 'center' })
}

function stepHit(delta: number): void {
  const total = hits.value.length
  if (total === 0) return
  matchIndex.value = (activeHit.value + delta + total) % total
  scrollToActive()
}

watch(query, (value) => {
  matchIndex.value = 0
  if (!value.trim()) {
    syncRawScroll()
    return
  }
  void nextTick(scrollToActive)
})

// 切视图后命中集合换了一套，从新视图的第一个命中重新开始。
watch(mode, () => {
  matchIndex.value = 0
  if (query.value.trim()) void nextTick(scrollToActive)
})

async function copy(): Promise<void> {
  const text = copyText.value
  if (!text) return
  try {
    await navigator.clipboard.writeText(text)
    copied.value = true
    if (copyTimer) clearTimeout(copyTimer)
    copyTimer = setTimeout(() => (copied.value = false), 1600)
  } catch {
    copied.value = false
  }
}

// 面板复用于多段正文：传入新文本时重置编辑内容、折叠与模式，但不改动视图习惯之外的东西。
watch(
  () => props.text,
  (text) => {
    source.value = text
    collapsed.value = new Set()
    copied.value = false
    mode.value = preferredMode(text)
    query.value = ''
    matchIndex.value = 0
  }
)

// 用户编辑后原文变成合法 JSON 时，若停在格式化视图则自然跟着重算；无需额外处理。
onBeforeUnmount(() => {
  if (copyTimer) clearTimeout(copyTimer)
})
</script>

<template>
  <div class="jv">
    <div class="jv-toolbar">
      <span v-if="label" class="jv-label">{{ label }}</span>
      <button class="jv-view" :class="{ on: mode === 'raw' }" @click="setMode('raw')">原文</button>
      <button
        class="jv-view"
        :class="{ on: mode === 'pretty' }"
        :disabled="!isJson"
        :title="isJson ? '格式化随意贴入的 JSON' : '不是合法 JSON，无法格式化'"
        @click="setMode('pretty')"
      >
        格式化
      </button>
      <template v-if="mode === 'raw' || isJson">
        <input
          v-model="query"
          class="jv-search mono"
          placeholder="搜索关键字"
          spellcheck="false"
          @keydown.enter.prevent="stepHit($event.shiftKey ? -1 : 1)"
          @keydown.esc="query = ''"
        />
        <span v-if="query.trim()" class="jv-search-count mono">
          {{ hits.length > 0 ? `${activeHit + 1}/${hits.length}` : '无匹配' }}
        </span>
        <button
          class="jv-step"
          :disabled="hits.length === 0"
          title="上一个匹配（Shift+Enter）"
          @click="stepHit(-1)"
        >
          ↑
        </button>
        <button
          class="jv-step"
          :disabled="hits.length === 0"
          title="下一个匹配（Enter）"
          @click="stepHit(1)"
        >
          ↓
        </button>
      </template>
      <template v-if="mode === 'pretty' && isJson">
        <span class="spacer" />
        <button class="btn btn-ghost btn-sm" title="展开全部节点" @click="expandAll">全部展开</button>
        <button class="btn btn-ghost btn-sm" title="收起全部节点" @click="collapseAll">全部收起</button>
      </template>
      <span v-else-if="!isJson" class="muted jv-note">非 JSON，仅显示原文</span>
      <span class="spacer" />
      <button
        class="btn btn-ghost btn-sm"
        :class="{ copied }"
        :title="mode === 'pretty' && isJson ? '复制格式化后的 JSON' : '复制原文'"
        @click="copy"
      >
        {{ copied ? '已复制' : '复制' }}
      </button>
    </div>

    <!-- 原文是可编辑的 textarea；高亮层垫在它下面，只画底色，文字仍由 textarea 显示，
         这样输入法组词、选区、光标都不受影响。两层滚动位置在 syncRawScroll 里同步。 -->
    <div v-if="mode === 'raw'" class="jv-edit-wrap">
      <pre ref="rawLayer" class="jv-edit jv-edit-layer mono" aria-hidden="true"><span v-for="(segment, si) in rawSegments" :key="si" :class="{ 'jv-hit': segment.hit >= 0, current: segment.hit === activeHit }">{{ segment.text }}</span></pre>
      <textarea
        ref="rawEditor"
        v-model="source"
        class="jv-edit jv-edit-input mono"
        spellcheck="false"
        @scroll="syncRawScroll"
      />
    </div>

    <div v-else ref="tree" class="jv-tree mono">
      <div
        v-for="(line, i) in lines"
        :key="i"
        class="jv-line"
        :data-line="i"
        :style="{ paddingLeft: line.depth * 14 + 'px' }"
      >
        <button
          v-if="isOpen(line)"
          class="jv-caret"
          :class="{ open: !isCollapsed(line.path) }"
          :title="isCollapsed(line.path) ? '展开节点' : '收起节点'"
          @click="toggle(line.path)"
        />
        <span v-else class="jv-caret-space" />

        <template v-if="line.key !== null">
          <span class="jv-key">"<span
            v-for="(segment, si) in rows[i].key"
            :key="si"
            :class="{ 'jv-hit': segment.hit >= 0, current: segment.hit === activeHit }"
          >{{ segment.text }}</span>"</span><span class="jv-punc">: </span>
        </template>

        <template v-if="isOpen(line)">
          <span class="jv-punc">{{ line.kind === 'open-array' ? '[' : '{' }}</span>
          <span v-if="isCollapsed(line.path)" class="jv-folded">
            … {{ line.count }} {{ line.kind === 'open-array' ? '项' : '个字段' }}
            <span class="jv-punc">{{ line.kind === 'open-array' ? ']' : '}' }}</span>
          </span>
        </template>
        <span v-else-if="line.kind.startsWith('close')" class="jv-punc">
          {{ line.kind === 'close-array' ? ']' : '}' }}
        </span>
        <span v-else class="jv-value" :class="`jv-${line.valueType}`"><span
          v-for="(segment, si) in rows[i].value"
          :key="si"
          :class="{ 'jv-hit': segment.hit >= 0, current: segment.hit === activeHit }"
        >{{ segment.text }}</span></span>

        <span v-if="showComma(line)" class="jv-punc">,</span>
      </div>
    </div>
  </div>
</template>
