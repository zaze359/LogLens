#!/usr/bin/env node
/**
 * Generate the default window icon that Tauri's `generate_context!()` reads at compile
 * time: `src-tauri/icons/icon.png`. The macro needs this file even when `bundle.icon`
 * is `[]`, so a fresh clone must have it before `cargo` runs.
 *
 * Zero dependencies, pure Node standard library. Idempotent: never overwrites an
 * existing icon (e.g. one produced by `npm run tauri icon <source.png>`).
 *
 * Output: a 64x64 RGBA PNG with a simple diagonal #38bdf8 -> #6366f1 gradient.
 */
const fs = require('fs')
const path = require('path')
const zlib = require('zlib')

const out = path.join(__dirname, '..', 'src-tauri', 'icons', 'icon.png')
if (fs.existsSync(out)) {
  process.exit(0)
}

const SIZE = 64

// CRC-32 (IEEE 802.3), required by the PNG chunk format.
const crcTable = new Uint32Array(256)
for (let n = 0; n < 256; n++) {
  let c = n
  for (let k = 0; k < 8; k++) {
    c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
  }
  crcTable[n] = c >>> 0
}
function crc32(buf) {
  let c = 0xffffffff
  for (const byte of buf) {
    c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8)
  }
  return (c ^ 0xffffffff) >>> 0
}
function chunk(type, data) {
  const typeBuf = Buffer.from(type, 'ascii')
  const length = Buffer.alloc(4)
  length.writeUInt32BE(data.length, 0)
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0)
  return Buffer.concat([length, typeBuf, data, crc])
}

// IHDR: width, height, bit depth 8, colour type 6 (RGBA), no interlace.
const ihdr = Buffer.alloc(13)
ihdr.writeUInt32BE(SIZE, 0)
ihdr.writeUInt32BE(SIZE, 4)
ihdr[8] = 8
ihdr[9] = 6
ihdr[10] = 0
ihdr[11] = 0
ihdr[12] = 0

// Scanlines: 1 filter byte (0 = None) + SIZE RGBA pixels.
const stride = SIZE * 4 + 1
const raw = Buffer.alloc(stride * SIZE)
for (let y = 0; y < SIZE; y++) {
  const rowStart = y * stride
  raw[rowStart] = 0
  for (let x = 0; x < SIZE; x++) {
    const o = rowStart + 1 + x * 4
    const t = (x + y) / (2 * (SIZE - 1))
    raw[o] = Math.round(56 + (99 - 56) * t)
    raw[o + 1] = Math.round(189 + (102 - 189) * t)
    raw[o + 2] = Math.round(248 + (241 - 248) * t)
    raw[o + 3] = 255
  }
}

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk('IHDR', ihdr),
  chunk('IDAT', zlib.deflateSync(raw)),
  chunk('IEND', Buffer.alloc(0))
])

fs.mkdirSync(path.dirname(out), { recursive: true })
fs.writeFileSync(out, png)
console.log(`[LogLens] generated default window icon: ${out} (${png.length} bytes)`)
