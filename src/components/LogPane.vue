<script setup lang="ts">
import RuleSidebar from './RuleSidebar.vue'
import FilterBar from './FilterBar.vue'
import StatsPanel from './StatsPanel.vue'
import LogTable from './LogTable.vue'
import LogDetail from './LogDetail.vue'
import { store } from '../stores/session'

/**
 * 日志面板本体：规则侧栏 + 筛选 + 表格 + 详情。
 *
 * 它同时服务两种来源 —— 本地文件工作区，和设备工作区里的「日志」工具。所以它只有一份实例
 * （显示器不重复挂载），靠 `v-show` 跟着上下文显隐：切到别的工具时只是隐藏，滚动位置、
 * 筛选、实时流都留着。
 */
</script>

<template>
  <div class="workspace">
    <RuleSidebar />
    <div class="main-pane">
      <FilterBar />
      <StatsPanel />
      <LogTable />
      <div class="statusbar">
        <span>已加载 <b class="stat-num">{{ store.entries.length }}</b> / 命中
          <b class="stat-num">{{ store.result?.matched ?? 0 }}</b> / 总计
          <b class="stat-num">{{ store.result?.total ?? 0 }}</b></span>
        <span v-if="store.result">耗时 {{ store.result.elapsedMs }} ms</span>
        <span v-if="store.fileInfo">未识别行 {{ store.fileInfo.unparsedLines }}</span>
        <span v-if="store.loadingMore" class="muted">加载更多中…</span>
        <div class="spacer" />
        <span class="muted">点击行查看完整内容 · 支持拖入日志文件 · 可直连 Android 设备</span>
      </div>
    </div>
    <LogDetail v-if="store.selected" />
  </div>
</template>
