<script setup lang="ts">
import { computed } from 'vue'
import {
  closeDevicePicker,
  openDeviceWorkspace,
  refreshDevices,
  setNotice,
  store
} from '../../stores/session'
import type { AdbDevice } from '../../types'

/**
 * Only job: pick which device the workbench acts on. Reached from the top bar's device
 * chip, i.e. when connecting or switching — not when opening a log/network panel.
 */
const ordered = computed(() =>
  [...store.devices].sort((a, b) => {
    const rank = (d: AdbDevice) => (d.state === 'device' ? 0 : 1)
    return rank(a) - rank(b) || a.serial.localeCompare(b.serial)
  })
)

function label(device: AdbDevice): string {
  return device.model || device.serial
}

function stateLabel(state: string): string {
  switch (state) {
    case 'device':
      return '已授权'
    case 'unauthorized':
      return '未授权'
    case 'offline':
      return '离线'
    case 'no permissions':
      return '无权限'
    default:
      return state || '未知'
  }
}

/** Picking a device creates its workspace (or focuses the one it already has). */
function choose(device: AdbDevice): void {
  if (device.state !== 'device') {
    setNotice(`${label(device)} 当前是「${stateLabel(device.state)}」，先在手机上授权再选择`)
    return
  }
  openDeviceWorkspace(device.serial)
  closeDevicePicker()
}

/** Already open? Say so, so the row reads as "switch to it" instead of "create another". */
function isOpen(serial: string): boolean {
  return store.deviceWorkspaces.includes(serial)
}
</script>

<template>
  <div class="modal-mask" @click.self="closeDevicePicker">
    <div class="modal device-picker-modal">
      <div class="modal-head">
        <span>连接设备</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" title="关闭" @click="closeDevicePicker">×</button>
      </div>

      <div class="modal-body">
        <div v-if="store.devicesLoading && !store.adbStatus" class="adb-line muted">
          <span class="dot-pending" />
          正在检测 adb（首次调用会启动 adb server，可能需要几秒）…
        </div>

        <div v-else-if="store.adbStatus && !store.adbStatus.available" class="adb-missing">
          <p class="adb-missing-title">未检测到 adb</p>
          <p class="empty-hint">{{ store.adbStatus.error }}</p>
          <p class="empty-hint">
            安装 Android SDK Platform-Tools 后，把 <code class="mono">platform-tools</code>
            目录加入 PATH；或设置环境变量 <code class="mono">ANDROID_HOME</code> /
            <code class="mono">ADB_PATH</code> 指到 adb 所在目录再重启客户端。
          </p>
        </div>

        <template v-else>
          <div class="adb-line">
            <span class="dot-ok" />
            <span class="mono">{{ store.adbStatus?.version || 'adb 已就绪' }}</span>
            <span class="muted mono ellipsis" :title="store.adbStatus?.path">
              {{ store.adbStatus?.path }}
            </span>
            <div class="spacer" />
            <button class="btn btn-sm" :disabled="store.devicesLoading" @click="refreshDevices()">
              {{ store.devicesLoading ? '刷新中…' : '刷新设备' }}
            </button>
          </div>

          <p v-if="ordered.length === 0" class="empty-hint">
            没有检测到设备。请用数据线连接手机，打开「开发者选项 → USB 调试」，并在手机弹窗里点「允许」。
            用「无线调试」时先在终端执行 <code class="mono">adb connect &lt;ip&gt;:&lt;端口&gt;</code>，再回来刷新。
          </p>

          <ul v-else class="device-list">
            <li v-for="device in ordered" :key="device.serial">
              <button
                class="device-item"
                :class="{ on: isOpen(device.serial), off: device.state !== 'device' }"
                @click="choose(device)"
              >
                <span class="dot-ok" v-if="device.state === 'device'" />
                <span class="dot-pending" v-else />
                <span class="device-item-name">{{ label(device) }}</span>
                <span class="mono muted">{{ device.serial }}</span>
                <div class="spacer" />
                <span class="device-item-state" :class="{ warn: device.state !== 'device' }">
                  {{ stateLabel(device.state) }}
                </span>
                <span v-if="isOpen(device.serial)" class="device-item-current">已打开</span>
              </button>
            </li>
          </ul>

          <p v-if="ordered.some((d) => d.state === 'unauthorized')" class="empty-hint warn">
            显示「未授权」的设备，解锁手机后在「允许 USB 调试吗？」弹窗里点允许，再刷新设备。
          </p>
          <p v-if="ordered.some((d) => d.state === 'offline')" class="empty-hint warn">
            显示「离线」的设备：重新插拔数据线，或执行 <code class="mono">adb reconnect</code> 后刷新。
          </p>
        </template>
      </div>

      <div class="modal-foot">
        <span class="empty-hint" style="margin-right: auto">
          选择后会在标签条里新建一个设备工作区，抓日志 / 抓包都在里面
        </span>
        <button class="btn btn-sm" @click="closeDevicePicker">关闭</button>
      </div>
    </div>
  </div>
</template>
