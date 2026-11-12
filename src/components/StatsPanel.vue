<script setup lang="ts">
import { computed } from 'vue'
import { store } from '../stores/session'

const result = computed(() => store.result)
const maxLevelCount = computed(() =>
  Math.max(1, ...(result.value?.levelCounts.map((c) => c.count) ?? [1]))
)

const colors: Record<string, string> = {
  V: '#64748b',
  D: '#60a5fa',
  I: '#34d399',
  W: '#fbbf24',
  E: '#f87171',
  F: '#ef4444',
  A: '#f472b6',
  S: '#a78bfa',
  '?': '#64748b'
}
</script>

<template>
  <div v-if="result" class="stats">
    <span>命中 <b class="stat-num">{{ result.matched.toLocaleString() }}</b> / {{ result.total.toLocaleString() }}</span>
    <div class="divider-v" />
    <div class="level-bars" title="命中日志的级别分布">
      <div v-for="c in result.levelCounts" :key="c.name" class="level-bar-item" :title="`${c.name}: ${c.count}`">
        <span class="mono muted">{{ c.name }}</span>
        <div class="level-bar" :style="{
          height: Math.max(3, (c.count / maxLevelCount) * 24) + 'px',
          background: colors[c.name] ?? '#64748b'
        }" />
        <span class="mono">{{ c.count }}</span>
      </div>
    </div>
    <div class="divider-v" />
    <div class="top-tags">
      <span class="muted">高频 Tag</span>
      <span v-for="t in result.topTags.slice(0, 5)" :key="t.name" class="chip mono">{{ t.name }} · {{ t.count }}</span>
    </div>
    <div class="spacer" />
    <span class="muted">{{ result.elapsedMs }} ms</span>
  </div>
  <div v-else class="stats">
    <span class="muted">打开日志文件后显示级别分布与高频 Tag 统计</span>
  </div>
</template>
