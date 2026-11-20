/**
 * JSON 正文查看器的纯逻辑：解析、格式化、摊平成可折叠的行。
 * 组件不重复实现解析规则；非 JSON 一律按原文展示，不猜测内容。
 *
 * 数字保留原文而不是交给 `JSON.parse`：抓包里的雪花 id（如
 * `7519341949566459904`）超过 `Number.MAX_SAFE_INTEGER`，转成 number 会变成
 * `7519341949566460000`，复制出去就是错的。
 */

/** 数字字面量的原文。解析与格式化都不做数值转换。 */
export class JsonNumber {
  constructor(readonly raw: string) {}
}

export type JsonPrimitive = null | boolean | string | JsonNumber
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue }
export type JsonValueType = 'string' | 'number' | 'boolean' | 'null' | 'object' | 'array'

export interface JsonParseResult {
  /** 解析成功即为 true；顶层标量也接受。 */
  ok: boolean
  value: JsonValue | null
  /** 缩进 2 空格的格式化文本（数字保持原文）；解析失败时原样返回。 */
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
  return value === null || typeof value !== 'object' || value instanceof JsonNumber
}

function primitiveType(value: JsonPrimitive): Exclude<JsonValueType, 'object' | 'array'> {
  if (value === null) return 'null'
  if (value instanceof JsonNumber) return 'number'
  if (typeof value === 'string') return 'string'
  return 'boolean'
}

function primitiveText(value: JsonPrimitive): string {
  if (value === null) return 'null'
  if (value instanceof JsonNumber) return value.raw
  if (typeof value === 'string') return JSON.stringify(value)
  return String(value)
}

/** 手写递归下降：只为保住数字原文，校验规则与 RFC 8259 一致。 */
class JsonParser {
  private index = 0

  constructor(private readonly text: string) {}

  parse(): JsonValue {
    const value = this.readValue()
    this.skipWhitespace()
    if (this.index < this.text.length) this.fail('JSON 之后还有多余内容')
    return value
  }

  private readValue(): JsonValue {
    this.skipWhitespace()
    const ch = this.text[this.index]
    if (ch === '{') return this.readObject()
    if (ch === '[') return this.readArray()
    if (ch === '"') return this.readString()
    if (ch === '-' || (ch !== undefined && ch >= '0' && ch <= '9')) return this.readNumber()
    for (const [literal, value] of [
      ['true', true],
      ['false', false],
      ['null', null]
    ] as const) {
      if (this.text.startsWith(literal, this.index)) {
        this.index += literal.length
        return value
      }
    }
    return this.fail('这里需要一个 JSON 值')
  }

  private readObject(): { [key: string]: JsonValue } {
    this.index++ // {
    // 无原型对象：键是 `__proto__` 时也不能改到原型链上去。
    const result: { [key: string]: JsonValue } = Object.create(null)
    this.skipWhitespace()
    if (this.text[this.index] === '}') {
      this.index++
      return result
    }
    while (true) {
      this.skipWhitespace()
      if (this.text[this.index] !== '"') this.fail('对象的键必须是字符串')
      const key = this.readString()
      this.skipWhitespace()
      if (this.text[this.index] !== ':') this.fail('键后面缺少冒号')
      this.index++
      result[key] = this.readValue()
      this.skipWhitespace()
      const ch = this.text[this.index]
      if (ch === ',') {
        this.index++
        continue
      }
      if (ch === '}') {
        this.index++
        return result
      }
      this.fail('对象里缺少逗号或右花括号')
    }
  }

  private readArray(): JsonValue[] {
    this.index++ // [
    const result: JsonValue[] = []
    this.skipWhitespace()
    if (this.text[this.index] === ']') {
      this.index++
      return result
    }
    while (true) {
      result.push(this.readValue())
      this.skipWhitespace()
      const ch = this.text[this.index]
      if (ch === ',') {
        this.index++
        continue
      }
      if (ch === ']') {
        this.index++
        return result
      }
      this.fail('数组里缺少逗号或右方括号')
    }
  }

  private readString(): string {
    const start = this.index
    this.index++ // 开头的引号
    while (this.index < this.text.length) {
      const ch = this.text[this.index]
      if (ch === '\\') {
        this.index += 2
        continue
      }
      if (ch === '"') {
        this.index++
        const token = this.text.slice(start, this.index)
        // 转义 / 控制字符交给 JSON.parse 校验，这里只负责找到字符串边界。
        try {
          return JSON.parse(token) as string
        } catch {
          return this.fail('字符串转义不合法')
        }
      }
      if (ch === '\n' || ch === '\r') this.fail('字符串里不能直接换行')
      this.index++
    }
    return this.fail('字符串没有闭合')
  }

  private readNumber(): JsonNumber {
    const match = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(this.text.slice(this.index))
    if (!match) this.fail('数字格式不正确')
    this.index += match[0].length
    return new JsonNumber(match[0])
  }

  private skipWhitespace(): void {
    while (this.index < this.text.length) {
      const ch = this.text[this.index]
      if (ch !== ' ' && ch !== '\t' && ch !== '\n' && ch !== '\r') return
      this.index++
    }
  }

  private fail(message: string): never {
    throw new Error(`${message}（第 ${this.index + 1} 个字符）`)
  }
}

/** 按 2 空格缩进输出，数字直接写原文，保持与输入完全一致的精度。 */
function formatJson(value: JsonValue, depth = 0): string {
  if (value instanceof JsonNumber) return value.raw
  if (value === null) return 'null'
  if (typeof value === 'string') return JSON.stringify(value)
  if (typeof value === 'boolean') return String(value)
  const pad = '  '.repeat(depth)
  const childPad = '  '.repeat(depth + 1)
  if (Array.isArray(value)) {
    if (value.length === 0) return '[]'
    const items = value.map((item) => `${childPad}${formatJson(item, depth + 1)}`)
    return `[\n${items.join(',\n')}\n${pad}]`
  }
  const entries = Object.entries(value)
  if (entries.length === 0) return '{}'
  const lines = entries.map(
    ([key, item]) => `${childPad}${JSON.stringify(key)}: ${formatJson(item, depth + 1)}`
  )
  return `{\n${lines.join(',\n')}\n${pad}}`
}

/** 去掉 BOM 与首尾空白后再解析；空串不算 JSON。 */
export function parseJsonBody(raw: string): JsonParseResult {
  const text = raw.replace(/^\uFEFF/, '').trim()
  if (!text) return { ok: false, value: null, formatted: raw, error: '' }
  try {
    const value = new JsonParser(text).parse()
    return {
      ok: true,
      value,
      formatted: formatJson(value),
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
