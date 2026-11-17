<script setup lang="ts">
import { closeJsonPanel, store } from '../stores/session'
import JsonViewer from './JsonViewer.vue'
</script>

<template>
  <div class="modal-mask" @click.self="closeJsonPanel">
    <div class="modal json-modal">
      <div class="modal-head">
        <span>JSON 格式化</span>
        <span v-if="store.jsonPanel.label" class="badge">{{ store.jsonPanel.label }}</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" title="关闭" @click="closeJsonPanel">×</button>
      </div>
      <div class="json-modal-body">
        <JsonViewer :text="store.jsonPanel.text" :label="store.jsonPanel.label" />
      </div>
      <div class="modal-foot">
        <span class="empty-hint" style="margin-right: auto">
          默认显示原文；可直接编辑或粘贴内容，点「格式化」查看分层 JSON
        </span>
        <button class="btn btn-sm" @click="closeJsonPanel">关闭</button>
      </div>
    </div>
  </div>
</template>
