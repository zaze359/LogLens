import type { DeviceOpId } from '../../stores/session'

export interface DeviceOperation {
  id: DeviceOpId
  label: string
  hint: string
}

/**
 * 一台设备下能做的事。这一层是「设备」和「它的内容（日志、请求表…）」之间的那一层：
 * 标签条第二行按这份列表渲染，`DevicePage` 按同一份列表决定显示哪个功能体。
 *
 * 加一项功能：这里加一条 → `DevicePage` 加一个 `v-show` 区块 → 会留在设备上的状态写进
 * `shutdownOperations()`。
 */
export const DEVICE_OPERATIONS: DeviceOperation[] = [
  {
    id: 'log',
    label: '抓日志',
    hint: '实时跟随 adb logcat，日志成为这台设备的子标签'
  },
  { id: 'network', label: '抓包', hint: 'H5 / 原生接口采集，或导入 HAR' }
]
