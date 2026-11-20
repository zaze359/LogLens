#!/usr/bin/env node
/**
 * 生成 tauri-plugin-updater 需要的 latest.json。
 *
 * 客户端请求 releases/latest/download/latest.json，按自己的 OS-ARCH 取对应条目，
 * 下载 URL 指向的文件必须与签名一一对应，所以这里只认「产物 + 同名 .sig」成对出现。
 *
 * 用法（CI 里由 release.yml 调用）：
 *   node scripts/make-latest-json.mjs \
 *     --dir dist-release --version 0.1.0 --tag v0.1.0 \
 *     --repo zaze359/LogLens [--notes "更新说明"] [--out dist-release/latest.json]
 */

import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

function arg(name, fallback = '') {
  const i = process.argv.indexOf(`--${name}`)
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback
}

const dir = arg('dir', '.')
const version = arg('version')
const tag = arg('tag')
const repo = arg('repo')
const notes = arg('notes', `LogLens ${version} 由 CI 自动构建。`)
const out = arg('out', join(dir, 'latest.json'))

if (!version || !tag || !repo) {
  console.error('缺少参数：--version / --tag / --repo 都是必填')
  process.exit(2)
}

/** 文件名 -> updater 的平台键（darwin-aarch64 / windows-x86_64 / linux-x86_64…）。 */
function platformKey(file) {
  const arch = (raw) => {
    if (raw === 'aarch64' || raw === 'arm64') return 'aarch64'
    if (raw === 'x64' || raw === 'amd64' || raw === 'x86_64') return 'x86_64'
    return ''
  }
  let m = file.match(/_([A-Za-z0-9_]+)\.app\.tar\.gz$/)
  if (m) {
    const a = arch(m[1])
    return a ? `darwin-${a}` : ''
  }
  m = file.match(/_([A-Za-z0-9_]+)\.AppImage$/)
  if (m) {
    const a = arch(m[1])
    return a ? `linux-${a}` : ''
  }
  m = file.match(/_([A-Za-z0-9_]+)-setup\.exe$/)
  if (m) {
    const a = arch(m[1])
    return a ? `windows-${a}` : ''
  }
  return ''
}

const files = readdirSync(dir).filter((f) => !f.endsWith('.sig') && !f.endsWith('.sha256'))
const platforms = {}
for (const file of files) {
  const key = platformKey(file)
  if (!key) continue
  let signature = ''
  try {
    signature = readFileSync(join(dir, `${file}.sig`), 'utf8').trim()
  } catch {
    console.error(`缺少签名文件：${file}.sig（${key}）`)
    process.exit(1)
  }
  platforms[key] = {
    signature,
    url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(file)}`
  }
  console.log(`  ${key.padEnd(16)} <- ${file}`)
}

const keys = Object.keys(platforms)
if (keys.length === 0) {
  console.error('没有找到可用的更新包（.app.tar.gz / .AppImage / *-setup.exe + .sig）')
  process.exit(1)
}

const payload = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms
}
writeFileSync(out, `${JSON.stringify(payload, null, 2)}\n`)
console.log(`已写入 ${out}（${keys.length} 个平台：${keys.join(', ')}）`)
