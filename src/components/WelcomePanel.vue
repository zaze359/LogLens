<script setup lang="ts">
import { computed } from 'vue'
import { openDevicePicker, openProxyPanel, pickAndLoadFile, store } from '../stores/session'

/** 引导页也显示独立代理状态：不打开日志、不选设备就能看到它在跑、在哪个地址。 */
const proxyLive = computed(() => !!store.proxyServer?.running)
const importBusy = computed(() => !!store.importTask)
</script>

<template>
  <section class="welcome">
    <div class="welcome-inner">
      <div class="brand-mark welcome-mark" aria-hidden="true">L</div>
      <h1 class="welcome-title">从一份日志开始</h1>
      <p class="welcome-sub">
        打开本地日志，或连上一台 Android 设备实时抓取。解析与筛选都在这台机器上完成。
      </p>

      <div class="welcome-actions">
        <button
          class="welcome-card"
          :disabled="importBusy"
          :title="importBusy ? '正在导入另一个日志' : '在新标签中打开日志文件（最多 4 个）'"
          @click="pickAndLoadFile"
        >
          <span class="welcome-card-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7">
              <path d="M6 3h8l4 4v14H6z" stroke-linejoin="round" />
              <path d="M14 3v5h4M9 12h6M9 16h6" stroke-linecap="round" />
            </svg>
          </span>
          <span class="welcome-card-title">{{ importBusy ? '导入中…' : '打开日志文件' }}</span>
          <span class="welcome-card-desc">支持 Logcat 文本，打开后可切换规则模板筛选</span>
          <span class="welcome-card-cta">选择文件 →</span>
        </button>

        <button
          class="welcome-card"
          title="选择一台 Android 设备，打开设备工作区（抓日志 / 抓包 / adb）"
          @click="openDevicePicker"
        >
          <span class="welcome-card-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7">
              <rect x="7" y="2" width="10" height="20" rx="2" />
              <path d="M10 5h4M10 19h4" stroke-linecap="round" />
            </svg>
          </span>
          <span class="welcome-card-title">连接设备</span>
          <span class="welcome-card-desc">通过 adb 实时抓取日志，或做网络抓包</span>
          <span class="welcome-card-cta">选择设备 →</span>
        </button>
      </div>

      <p class="welcome-tip">也可以把日志文件直接拖进窗口 · 最多同时查看 4 份日志</p>

      <div class="welcome-proxy" :class="{ live: proxyLive }">
        <template v-if="proxyLive">
          <span class="live-dot" />
          <span>独立代理运行中 · <b class="mono">{{ store.proxyServer?.endpoint }}</b> ·
            已接入 {{ store.proxyServer?.sources ?? 0 }} 个来源</span>
          <button class="btn btn-ghost btn-sm" @click="openProxyPanel">查看请求</button>
        </template>
        <template v-else>
          <span class="muted">独立代理未开启：任何把代理指向本机的设备都能被抓包</span>
          <button class="btn btn-sm" @click="openProxyPanel">开启代理</button>
        </template>
      </div>
    </div>
  </section>
</template>
