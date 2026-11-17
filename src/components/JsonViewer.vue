<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { flattenJson, parseJsonBody } from '../lib/json'
import type { JsonLine } from '../lib/json'

/**
 * 通用 JSON 查看器：默认显示原文且可直接编辑 / 粘贴，点「格式化」切到语法着色的树视图，
 * 支持逐节点折叠与复制。不是合法 JSON 时格式化按钮不可用，始终按原文展示、不改动内容。
 * 抓包详情里只放一个「格式化」按钮，点开独立面板复用本组件。
 */
const props = defineProps<{ text: string; label: string }>()

type ViewMode = 'raw' | 'pretty'
/** 默认原文：打开面板先看到后端给的原样字符串，用户自己决定要不要格式化。 */
const mode = ref<ViewMode>('raw')
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

/** 展开时逗号跟闭合行，折叠时闭合行被隐藏，逗号改挂在开口行上。 */
function showComma(line: JsonLine): boolean {
  if (!line.comma) return false
  return line.kind.startsWith('close') || (isOpen(line) && isCollapsed(line.path))
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
    mode.value = 'raw'
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

    <textarea v-if="mode === 'raw'" v-model="source" class="jv-edit mono" spellcheck="false" />

    <div v-else class="jv-tree mono">
      <div
        v-for="(line, i) in lines"
        :key="i"
        class="jv-line"
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
          <span class="jv-key">"{{ line.key }}"</span><span class="jv-punc">: </span>
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
        <span v-else class="jv-value" :class="`jv-${line.valueType}`">{{ line.text }}</span>

        <span v-if="showComma(line)" class="jv-punc">,</span>
      </div>
    </div>
  </div>
</template>
