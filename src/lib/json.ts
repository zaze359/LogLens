/**
 * JSON 正文查看器的纯逻辑：解析、格式化、摊平成可折叠的行。
 * 组件不重复实现解析规则；非 JSON 一律按原文展示，不猜测内容。
 */

export type JsonPrimitive = null | boolean | number | string
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue }
export type JsonValueType = 'string' | 'number' | 'boolean' | 'null' | 'object' | 'array'

export interface JsonParseResult {
  /** JSON.parse 成功即为 true；顶层标量也接受。 */
  ok: boolean
  value: JsonValue | null
  /** 缩进 2 空格的格式化文本；解析失败时原样返回。 */
  formatted: string
  error: string
}

export interface JsonLine {
  /** 容器行的唯一路径，折叠状态以它为键。 */
  path: string
  depth: number
  kind: 'open-object' | 'open-array' | 'close-object' | 'close-array' | 'value'
  /** 属性名；数组元素与根节点为 null。 */
  key: string | null
  /** 标量行的显示文本，字符串已按 JSON 规则转义。 */
  text: string
  valueType: JsonValueType
  /** 容器的直接子项数量。 */
  count: number
  /** 行尾是否需要逗号。 */
  comma: boolean
  /** 祖先容器的 path，用于判断该行是否被折叠隐藏。 */
  ancestors: string[]
}

function isPrimitive(value: JsonValue): value is JsonPrimitive {
  return value === null || typeof value !== 'object'
}

function primitiveType(value: JsonPrimitive): Exclude<JsonValueType, 'object' | 'array'> {
  if (value === null) return 'null'
  if (typeof value === 'string') return 'string'
  if (typeof value === 'number') return 'number'
  return 'boolean'
}

function primitiveText(value: JsonPrimitive): string {
  if (value === null) return 'null'
  if (typeof value === 'string') return JSON.stringify(value)
  return String(value)
}

/** 去掉 BOM 与首尾空白后再解析；空串不算 JSON。 */
export function parseJsonBody(raw: string): JsonParseResult {
  const text = raw.replace(/^\uFEFF/, '').trim()
  if (!text) return { ok: false, value: null, formatted: raw, error: '' }
  try {
    const value = JSON.parse(text) as JsonValue
    return {
      ok: true,
      value,
      formatted: JSON.stringify(value, null, 2) ?? text,
      error: ''
    }
  } catch (e) {
    return {
      ok: false,
      value: null,
      formatted: raw,
      error: e instanceof Error ? e.message : String(e)
    }
  }
}

/**
 * 把树摊平成行，避免递归组件传折叠状态。
 * 每个容器各占一对 open / close 行，close 行带逗号（展开时显示，折叠时隐藏）。
 */
export function flattenJson(value: JsonValue): JsonLine[] {
  const lines: JsonLine[] = []

  const visit = (
    node: JsonValue,
    key: string | null,
    depth: number,
    path: string,
    ancestors: string[],
    comma: boolean
  ): void => {
    if (!isPrimitive(node)) {
      const isArray = Array.isArray(node)
      const items: Array<[string | null, JsonValue]> = isArray
        ? (node as JsonValue[]).map((item) => [null, item])
        : Object.entries(node as Record<string, JsonValue>)
      const valueType: JsonValueType = isArray ? 'array' : 'object'
      const childAncestors = [...ancestors, path]

      lines.push({
        path, depth, key, text: '', valueType, count: items.length,
        comma, ancestors, kind: isArray ? 'open-array' : 'open-object'
      })
      items.forEach(([childKey, childValue], index) => {
        const label = isArray ? String(index) : (childKey as string)
        visit(
          childValue,
          childKey,
          depth + 1,
          `${path}/${encodeURIComponent(label)}`,
          childAncestors,
          index < items.length - 1
        )
      })
      lines.push({
        path, depth, key: null, text: '', valueType, count: items.length,
        comma, ancestors, kind: isArray ? 'close-array' : 'close-object'
      })
      return
    }

    lines.push({
      path, depth, key, text: primitiveText(node), valueType: primitiveType(node),
      count: 0, comma, ancestors, kind: 'value'
    })
  }

  visit(value, null, 0, '$', [], false)
  return lines
}
