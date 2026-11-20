/**
 * 日志与抓包共用 `HH:MM:SS.mmm` 作为时间轴键，这两个转换只处理当日时间 ——
 * 采集与导入的时间戳都不带日期，跨天的情况交给调用方按各自语义处理。
 */

/** `HH:MM:SS.mmm` → 当日毫秒；解析失败返回 null（不猜日期）。 */
export function timeMillis(key: string): number | null {
  const matched = /^(\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,3}))?$/.exec(key || '')
  if (!matched) return null
  const [h, m, s] = [Number(matched[1]), Number(matched[2]), Number(matched[3])]
  return (h * 3600 + m * 60 + s) * 1000 + Number((matched[4] || '0').padEnd(3, '0'))
}

/** 当日毫秒 → `HH:MM:SS.mmm`。 */
export function formatMillis(value: number): string {
  const h = Math.floor(value / 3600000)
  const m = Math.floor((value % 3600000) / 60000)
  const s = Math.floor((value % 60000) / 1000)
  const ms = Math.floor(value % 1000)
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}.${String(ms).padStart(3, '0')}`
}

function pad2(value: number): string {
  return String(value).padStart(2, '0')
}

/** 把手输的时间（`10:23`、`10:23:45,12`）补成 `HH:MM:SS.mmm`，与后端的归一化一致。 */
export function normalizeTimeKey(raw: string): string {
  const tail = raw.trim().replace(',', '.').split(/\s+/).pop() ?? ''
  if (!tail) return ''
  const [base, frac = ''] = tail.split('.')
  const parts = base.split(':')
  while (parts.length < 3) parts.push('0')
  return `${parts[0].padStart(2, '0')}:${parts[1].padStart(2, '0')}:${parts[2].padStart(2, '0')}.${(frac + '000').slice(0, 3)}`
}

/** 轴上的毫秒 → 标签；是否带日期、带年份由时间轴本身决定。 */
export function formatTimelineValue(ms: number, hasDate: boolean, hasYear: boolean): string {
  const date = new Date(ms)
  const time = `${pad2(date.getUTCHours())}:${pad2(date.getUTCMinutes())}:${pad2(date.getUTCSeconds())}.${String(
    date.getUTCMilliseconds()
  ).padStart(3, '0')}`
  if (!hasDate) return time
  const monthDay = `${pad2(date.getUTCMonth() + 1)}-${pad2(date.getUTCDate())}`
  return hasYear ? `${date.getUTCFullYear()}-${monthDay} ${time}` : `${monthDay} ${time}`
}

/** `2026-11-12` → 真实年份；`11-12` → 占位年份 1970（与后端的归一化一致）。 */
function parseDatePart(raw: string): { y: number; m: number; d: number } | null {
  const full = /^(\d{4})-(\d{2})-(\d{2})$/.exec(raw)
  if (full) {
    const [y, m, d] = [Number(full[1]), Number(full[2]), Number(full[3])]
    return m >= 1 && m <= 12 && d >= 1 && d <= 31 ? { y, m, d } : null
  }
  const short = /^(\d{2})-(\d{2})$/.exec(raw)
  if (short) {
    const [m, d] = [Number(short[1]), Number(short[2])]
    return m >= 1 && m <= 12 && d >= 1 && d <= 31 ? { y: 1970, m, d } : null
  }
  return null
}

/**
 * 用户输入的区间端点 → 轴上的毫秒。
 *
 * 轴带日期时，只给时刻的输入无法定位到某一天，返回 `null`（不画区间高亮，但仍照常
 * 参与后端筛选 —— 那是有意保留的「每天同一时刻」语义）。
 */
export function parseTimeValue(raw: string, axisHasDate: boolean): number | null {
  const cleaned = raw.trim().replace(',', '.')
  if (!cleaned) return null
  const [head, timePart] = cleaned.split(/\s+/, 2)
  if (timePart !== undefined) {
    const date = parseDatePart(head)
    const tod = timeMillis(normalizeTimeKey(timePart))
    if (!date || tod === null) return null
    return Date.UTC(date.y, date.m - 1, date.d) + tod
  }
  if (axisHasDate) return null
  return timeMillis(normalizeTimeKey(cleaned))
}
