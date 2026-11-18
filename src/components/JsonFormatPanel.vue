<script setup lang="ts">
import { ref } from 'vue'
import { closeJsonPanel, store } from '../stores/session'
import JsonViewer from './JsonViewer.vue'

/**
 * 全屏只放大到「铺满应用窗口」，不用浏览器 Fullscreen API：桌面端那个会把窗口顶到
 * 系统级全屏，退出还得靠系统手势或 Esc，反而更难用。面板用 v-if 挂载，关掉再开
 * 会自然回到普通尺寸。
 */
const fullscreen = ref(false)
</script>

<template>
  <!-- 不响应遮罩点击：面板里可以直接编辑，点外面一下就把输入丢了太容易误触。
       关闭只保留右上角的 ×。 -->
  <div class="modal-mask" :class="{ 'is-fullscreen': fullscreen }">
    <div class="modal json-modal" :class="{ 'is-fullscreen': fullscreen }">
      <div class="modal-head">
        <span>JSON 格式化</span>
        <span v-if="store.jsonPanel.label" class="badge">{{ store.jsonPanel.label }}</span>
        <div class="spacer" />
        <button
          class="btn btn-ghost btn-sm"
          :title="fullscreen ? '还原窗口' : '全屏显示'"
          @click="fullscreen = !fullscreen"
        >
          {{ fullscreen ? '还原' : '全屏' }}
        </button>
        <button class="btn btn-ghost btn-sm" title="关闭" @click="closeJsonPanel">×</button>
      </div>
      <div class="json-modal-body">
        <JsonViewer :text="store.jsonPanel.text" :label="store.jsonPanel.label" />
      </div>
      <div class="modal-foot">
        <span class="empty-hint" style="margin-right: auto">
          默认显示原文；可直接编辑或粘贴内容，点「格式化」查看分层 JSON
        </span>
      </div>
    </div>
  </div>
</template>
