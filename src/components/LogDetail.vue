<script setup lang="ts">
import { computed } from 'vue'
import { LEVEL_LABELS } from '../types'
import { setNotice, store } from '../stores/session'

const entry = computed(() => store.selected)

async function copyRaw(): Promise<void> {
  if (!entry.value) return
  try {
    await navigator.clipboard.writeText(entry.value.raw)
    setNotice('已复制原始日志')
  } catch {
    setNotice('复制失败，请手动选中下方文本')
  }
}
</script>

<template>
  <aside v-if="entry" class="drawer">
    <div class="drawer-head">
      <span class="badge-level" :class="`lv-${entry.level === '?' ? 'unknown' : entry.level.toLowerCase()}`">
        {{ entry.level }}
      </span>
      <strong>日志详情</strong>
      <span class="muted mono">#{{ entry.index }}</span>
      <div class="spacer" />
      <button class="btn btn-sm" @click="copyRaw">复制原始行</button>
      <button class="btn btn-ghost btn-sm" @click="store.selected = null">关闭</button>
    </div>
    <div class="drawer-body">
      <dl class="kv">
        <dt>级别</dt><dd>{{ LEVEL_LABELS[entry.level] ?? entry.level }}</dd>
        <dt>时间</dt><dd class="mono">{{ entry.timestamp || '—' }}</dd>
        <dt>Tag</dt><dd class="mono">{{ entry.tag || '—' }}</dd>
        <dt>PID / TID</dt><dd class="mono">{{ entry.pid || '—' }} / {{ entry.tid || '—' }}</dd>
        <dt>包名</dt><dd class="mono">{{ entry.package || '—' }}</dd>
        <dt>组件类名</dt><dd class="mono">{{ entry.component || '—' }}</dd>
        <dt>附加行</dt><dd>{{ entry.continuation }}</dd>
        <dt>解析状态</dt><dd>{{ entry.parseOk ? '已识别' : '未识别（原样保留）' }}</dd>
      </dl>
      <div class="side-title">原始内容（含堆栈等续行）</div>
      <pre class="raw-box">{{ entry.raw }}</pre>
    </div>
  </aside>
</template>
