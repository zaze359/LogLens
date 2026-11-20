<script setup lang="ts">
/**
 * 日志时间轴：把命中日志按时间分桶画成密度图，拖拽即可把区间写回筛选条件。
 *
 * 数据来自后端 `log_timeline`：轴的范围取自整份日志，计数只认命中结果，且统计时忽略
 * `timeFrom` / `timeTo` —— 选中一段区间后，其他时间段的分布仍然留在图上。
 */
import { computed, ref, watch } from 'vue'
import { formatTimelineValue, parseTimeValue } from '../lib/time'
import { store, updateFilter } from '../stores/session'

const chart = ref<HTMLElement | null>(null)

const timeline = computed(() => store.timeline)

/** 轴的首尾（绝对毫秒）。只有一个时间点时把终点撑开 1ms，避免除零。 */
const axis = computed(() => {
  const t = timeline.value
  if (!t || t.bins.length === 0) return null
  return { start: t.startMs, end: Math.max(t.endMs, t.startMs + 1) }
})

const bars = computed(() => {
  const t = timeline.value
  const bounds = axis.value
  if (!t || !bounds) return []
  const max = Math.max(1, ...t.bins)
  const span = bounds.end - bounds.start
  const width = 100 / t.bins.length
  return t.bins.map((count, i) => {
    const from = Math.round(bounds.start + (span * i) / t.bins.length)
    const to = Math.round(bounds.start + (span * (i + 1)) / t.bins.length)
    return {
      key: i,
      left: i * width,
      width,
      height: count > 0 ? Math.max(2, (count / max) * 100) : 0,
      count,
      title: `${formatTimelineValue(from, t.hasDate, t.hasYear)} – ${formatTimelineValue(to, t.hasDate, t.hasYear)} · ${count} 条`
    }
  })
})

/** 当前筛选条件对应到轴上的区间；只填了一端就延伸到轴的另一端。 */
const activeRange = computed(() => {
  const t = timeline.value
  const bounds = axis.value
  if (!t || !bounds) return null
  const from = store.filter.timeFrom ? parseTimeValue(store.filter.timeFrom, t.hasDate) : null
  const to = store.filter.timeTo ? parseTimeValue(store.filter.timeTo, t.hasDate) : null
  if (from === null && to === null) return null
  // 输入落在轴外（例如只写了月日、而日志带真实年份）时画不出正确位置，宁可不画高亮。
  const outside = (value: number | null): boolean =>
    value !== null && (value < bounds.start || value > bounds.end)
  if (outside(from) || outside(to)) return null
  return { start: from ?? bounds.start, end: to ?? bounds.end }
})

/** 只要填了时间条件就认为区间在生效（哪怕它无法在轴上定位）。 */
const hasTimeFilter = computed(() => !!(store.filter.timeFrom || store.filter.timeTo))

function percentOf(ms: number): number {
  const bounds = axis.value
  if (!bounds) return 0
  return Math.min(100, Math.max(0, ((ms - bounds.start) / (bounds.end - bounds.start)) * 100))
}

const selectedBand = computed(() => {
  const range = activeRange.value
  if (!range) return null
  const left = percentOf(range.start)
  const right = percentOf(range.end)
  return { left, width: Math.max(0.6, right - left) }
})

/* 拖拽状态用 0–1 的轴比例保存，指针事件换算与渲染都只依赖百分比。 */
const dragFrom = ref<number | null>(null)
const dragTo = ref<number | null>(null)

const dragBand = computed(() => {
  if (dragFrom.value === null || dragTo.value === null) return null
  const from = Math.min(dragFrom.value, dragTo.value) * 100
  const to = Math.max(dragFrom.value, dragTo.value) * 100
  return { left: from, width: to - from }
})

function ratioAt(event: PointerEvent): number | null {
  const el = chart.value
  if (!el) return null
  const rect = el.getBoundingClientRect()
  if (rect.width <= 0) return null
  return Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width))
}

function onDown(event: PointerEvent): void {
  if (!axis.value) return
  const ratio = ratioAt(event)
  if (ratio === null) return
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  dragFrom.value = ratio
  dragTo.value = ratio
}

function onMove(event: PointerEvent): void {
  if (dragFrom.value === null) return
  dragTo.value = ratioAt(event)
}

function onUp(): void {
  const from = dragFrom.value
  const to = dragTo.value
  const bounds = axis.value
  const t = timeline.value
  dragFrom.value = null
  dragTo.value = null
  if (from === null || to === null || !bounds) return
  // 几乎没拖动就是「点了一下」：点在柱子上就定格这根柱子的区间，点在空白桶上则清除。
  const width = chart.value?.getBoundingClientRect().width ?? 0
  if (Math.abs(to - from) * width < 4) {
    if (!t || t.bins.length === 0) return
    const index = Math.min(t.bins.length - 1, Math.max(0, Math.floor(from * t.bins.length)))
    if (t.bins[index] > 0) selectBin(index)
    else clearRange()
    return
  }
  if (!t) return
  const span = bounds.end - bounds.start
  const start = Math.round(bounds.start + Math.min(from, to) * span)
  const end = Math.round(bounds.start + Math.max(from, to) * span)
  updateFilter({
    timeFrom: formatTimelineValue(start, t.hasDate, t.hasYear),
    timeTo: formatTimelineValue(end, t.hasDate, t.hasYear)
  })
}

/** 把第 `index` 个桶的起止时间写回筛选条件，柱状图上的区间随之定格在这一格。 */
function selectBin(index: number): void {
  const t = timeline.value
  const bounds = axis.value
  if (!t || !bounds) return
  const span = bounds.end - bounds.start
  const start = Math.round(bounds.start + (span * index) / t.bins.length)
  const end = Math.round(bounds.start + (span * (index + 1)) / t.bins.length)
  updateFilter({
    timeFrom: formatTimelineValue(start, t.hasDate, t.hasYear),
    timeTo: formatTimelineValue(end, t.hasDate, t.hasYear)
  })
}

function clearRange(): void {
  updateFilter({ timeFrom: '', timeTo: '' })
}

/**
 * 手输时间与拖拽共用同一份筛选状态：输入框只是它的文本镜像。
 * 值没变就不回写，免得拖拽写回的区间再触发一次多余刷新。
 */
const timeFrom = ref(store.filter.timeFrom)
const timeTo = ref(store.filter.timeTo)

watch(() => store.filter.timeFrom, (v) => { if (v !== timeFrom.value) timeFrom.value = v })
watch(() => store.filter.timeTo, (v) => { if (v !== timeTo.value) timeTo.value = v })
watch(timeFrom, (v) => { if (v !== store.filter.timeFrom) updateFilter({ timeFrom: v }, true) })
watch(timeTo, (v) => { if (v !== store.filter.timeTo) updateFilter({ timeTo: v }, true) })

/* ---------- 时间选择器：用原生 date / time 控件补全年月日 ---------- */

const pickerFor = ref<'from' | 'to' | ''>('')
const pickDate = ref('')
const pickTime = ref('')

function pad2(value: number): string {
  return String(value).padStart(2, '0')
}

/** 打开选择器时用当前端点（没有就用轴端点）作初值。 */
function openPicker(which: 'from' | 'to'): void {
  const t = timeline.value
  if (!t) return
  const raw = which === 'from' ? store.filter.timeFrom : store.filter.timeTo
  const parsed = raw ? parseTimeValue(raw, t.hasDate) : null
  const inAxis = parsed !== null && parsed >= t.startMs && parsed <= t.endMs
  const base = inAxis ? (parsed as number) : which === 'from' ? t.startMs : t.endMs
  const date = new Date(base)
  pickDate.value = `${date.getUTCFullYear()}-${pad2(date.getUTCMonth() + 1)}-${pad2(date.getUTCDate())}`
  pickTime.value = `${pad2(date.getUTCHours())}:${pad2(date.getUTCMinutes())}:${pad2(date.getUTCSeconds())}`
  pickerFor.value = which
}

function applyPicker(): void {
  const which = pickerFor.value
  if (!which || !pickDate.value) return
  // `type=time` 在部分平台只给到分钟，补齐秒。
  const time = pickTime.value ? (pickTime.value.length === 5 ? `${pickTime.value}:00` : pickTime.value) : '00:00:00'
  const value = `${pickDate.value} ${time}`
  updateFilter(which === 'from' ? { timeFrom: value } : { timeTo: value })
  pickerFor.value = ''
}
</script>

<template>
  <div v-if="timeline && timeline.bins.length" class="log-timeline">
    <div class="log-timeline-head">
      <span class="muted">时间轴</span>
      <span class="mono muted">{{ timeline.start }} → {{ timeline.end }}</span>
      <span class="muted">分布 {{ timeline.total.toLocaleString() }} 条</span>
      <span v-if="timeline.untimed > 0" class="muted">{{ timeline.untimed }} 条无时间戳</span>
      <div class="spacer" />
      <span class="filter-label">时间</span>
      <input v-model="timeFrom" class="input mono log-time-input" placeholder="YYYY-MM-DD HH:MM:SS" />
      <button
        v-if="timeline?.hasDate"
        class="btn btn-sm log-time-pick"
        title="选择起始时间"
        @click="openPicker('from')"
      >
        ▾
      </button>
      <span class="muted">~</span>
      <input v-model="timeTo" class="input mono log-time-input" placeholder="YYYY-MM-DD HH:MM:SS" />
      <button
        v-if="timeline?.hasDate"
        class="btn btn-sm log-time-pick"
        title="选择结束时间"
        @click="openPicker('to')"
      >
        ▾
      </button>
      <span v-if="hasTimeFilter" class="log-range-label mono">
        命中 {{ (store.result?.matched ?? 0).toLocaleString() }} 条
        <button class="btn btn-ghost btn-sm" @click="clearRange">清除区间</button>
      </span>
      <span v-else class="muted">拖拽 / 点击柱状选择区间</span>
      <div v-if="pickerFor" class="log-time-picker">
        <span class="muted">{{ pickerFor === 'from' ? '起始' : '结束' }}</span>
        <input v-model="pickDate" type="date" class="input" />
        <input v-model="pickTime" type="time" step="1" class="input" />
        <button class="btn btn-sm btn-primary" @click="applyPicker">确定</button>
        <button class="btn btn-ghost btn-sm" @click="pickerFor = ''">取消</button>
      </div>
    </div>
    <div
      ref="chart"
      class="log-timeline-chart"
      title="拖拽选择时间区间；点击柱状定格该区间，点击空白清除区间"
      @pointerdown="onDown"
      @pointermove="onMove"
      @pointerup="onUp"
      @pointercancel="onUp"
    >
      <span
        v-for="bar in bars"
        :key="bar.key"
        class="log-tl-bar"
        :class="{ empty: bar.count === 0 }"
        :style="{ left: bar.left + '%', width: bar.width + '%', height: bar.height + '%' }"
        :title="bar.title"
      />
      <span
        v-if="selectedBand"
        class="log-tl-band"
        :style="{ left: selectedBand.left + '%', width: selectedBand.width + '%' }"
      />
      <span
        v-if="dragBand"
        class="log-tl-band dragging"
        :style="{ left: dragBand.left + '%', width: dragBand.width + '%' }"
      />
    </div>
  </div>
</template>
