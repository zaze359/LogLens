<script setup lang="ts">
import { computed } from 'vue'
import { checkUpdate, closeUpdatePanel, installUpdate, store } from '../stores/session'

/** 进度条宽度：服务器没给总长度时退化成不确定进度（0%，但按钮文案说明在下载）。 */
const percent = computed<number>(() => {
  const { downloaded, total } = store.update
  if (!total) return 0
  return Math.min(100, Math.round((downloaded / total) * 100))
})

function mb(bytes: number): string {
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

const progressText = computed<string>(() => {
  const { downloaded, total } = store.update
  if (!total) return '下载中…'
  return `${mb(downloaded)} / ${mb(total)} · ${percent.value}%`
})

const idleHint = computed<string>(() => {
  if (store.update.checked) return '当前已是最新版本。'
  return '启动时会自动检查一次；也可以现在手动检查。'
})
</script>

<template>
  <div class="modal-mask" @click.self="closeUpdatePanel">
    <div class="modal modal-sm update-modal">
      <div class="modal-head">
        <span>软件更新</span>
        <span v-if="store.update.available" class="badge">有新版本</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" title="关闭更新面板" @click="closeUpdatePanel">
          关闭
        </button>
      </div>

      <div class="modal-body">
        <div class="form-row">
          <span class="form-label">当前版本</span>
          <span class="mono">{{ store.appVersion || '—' }}</span>
        </div>
        <div v-if="store.update.available" class="form-row">
          <span class="form-label">新版本</span>
          <span class="mono">{{ store.update.available.version }}</span>
          <span v-if="store.update.available.date" class="muted">
            {{ store.update.available.date }}
          </span>
        </div>

        <p v-if="store.update.checking" class="empty-hint">正在检查更新…</p>

        <template v-else-if="store.update.available">
          <p v-if="store.update.available.notes" class="empty-hint update-notes">
            {{ store.update.available.notes }}
          </p>
          <div v-if="store.update.downloading || store.update.installed" class="update-progress">
            <div class="update-progress-bar">
              <span :style="{ width: `${percent}%` }" />
            </div>
            <span class="muted mono">{{ progressText }}</span>
          </div>
          <p v-if="store.update.installed" class="empty-hint">安装完成，正在重启…</p>
        </template>

        <p v-else class="empty-hint">{{ idleHint }}</p>

        <p v-if="store.update.error" class="empty-hint error-text">{{ store.update.error }}</p>
      </div>

      <div class="modal-foot">
        <button
          class="btn btn-sm"
          :disabled="store.update.checking || store.update.downloading"
          @click="checkUpdate(true)"
        >
          {{ store.update.checking ? '检查中…' : '检查更新' }}
        </button>
        <button
          v-if="store.update.available"
          class="btn btn-primary btn-sm"
          :disabled="store.update.downloading || store.update.installed"
          @click="installUpdate"
        >
          <template v-if="store.update.installed">重启中…</template>
          <template v-else-if="store.update.downloading">下载中…</template>
          <template v-else>下载并安装 {{ store.update.available.version }}</template>
        </button>
      </div>
    </div>
  </div>
</template>
