<script setup lang="ts">
import { computed } from 'vue'
import { setDeviceOp, store } from '../../stores/session'
import { DEVICE_OPERATIONS } from './operations'

/**
 * 设备工作区左边那条工具栏 —— 一台设备能做的事都在这儿，点图标切，右侧看内容。
 *
 * 日志、抓包不是「另一个工作区」，而是这台设备的工具：所以它们不进标签条（标签条上是
 * 工作区），关掉设备时它们一起消失，也不存在「设备已经没了、日志标签还挂着」的状态。
 *
 * 加工具 = `operations.ts` 加一条 + 这里加一个图标。
 */
const props = defineProps<{ serial: string }>()

const current = computed<string>(() => store.deviceOps[props.serial] ?? 'log')
</script>

<template>
  <nav class="tool-rail" :title="serial">
    <button
      v-for="op in DEVICE_OPERATIONS"
      :key="op.id"
      class="tool-btn"
      :class="{ on: current === op.id }"
      :title="op.label"
      :aria-label="op.label"
      @click="setDeviceOp(serial, op.id)"
    >
      <!-- 日志：一条条往下走的日志流 -->
      <svg v-if="op.id === 'log'" viewBox="0 0 20 20" aria-hidden="true">
        <path d="M3.5 4.5h13M3.5 8h9M3.5 11.5h11M3.5 15h6" />
      </svg>
      <!-- 抓包：一张网（节点 + 连线） -->
      <svg v-else-if="op.id === 'network'" viewBox="0 0 20 20" aria-hidden="true">
        <circle cx="10" cy="10" r="6.6" />
        <path d="M3.6 10h12.8M10 3.4c2 2 2 11.2 0 13.2M10 3.4c-2 2-2 11.2 0 13.2" />
      </svg>
      <!-- 未知工具：留一个占位方框，不静默漏掉 -->
      <svg v-else viewBox="0 0 20 20" aria-hidden="true">
        <rect x="4" y="4" width="12" height="12" rx="2.5" />
      </svg>
    </button>
  </nav>
</template>
