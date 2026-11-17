# LogLens Logo

日志分析客户端 LogLens 的应用图标。

## 设计概念

延续 Android Studio logcat 图标的视觉语言：

- **蓝色圆角方底** — 沿用现有品牌色（`#4C9BFF → #2B67E0`），也是 logcat 图标的标志性容器。
- **Android 机器人探头** — 从日志面板后探出，直接点明日志来源与 Android 场景。
- **三条彩色日志行** — 红 / 黄 / 绿对应 Error / Warn / Info，一眼能认出这是日志查看工具。

## 文件

| 路径 | 用途 |
| --- | --- |
| `logo-final.svg` | 矢量源文件，可读的几何定义，改设计时从这里入手 |
| `render_icon.py` | 零依赖渲染脚本（仅 Pillow），4x 超采样，输出全部尺寸 |
| `out/icon-*.png` | 16 / 32 / 48 / 64 / 128 / 256 / 512 / 1024 多尺寸 PNG |
| `out/icon.icns` | macOS 图标 |
| `out/icon.ico` | Windows 图标（含 6 种分辨率） |
| `out/icon.iconset/` | macOS 图标集，可重新 `iconutil -c icns` |
| `logo-a-logcat.svg` / `logo-b-focus.svg` / `logo-c-monogram.svg` | 早期三套备选方案（致敬 / 透镜 / 极简字母），未采用 |

## 重新生成

```bash
cd design/logo
python3 render_icon.py
iconutil -c icns out/icon.iconset -o out/icon.icns
```

## 已接入位置

`src-tauri/icons/` 下的 `icon.png`、`32x32.png`、`128x128.png`、`128x128@2x.png`、`icon.icns`、`icon.ico` 已替换；
打包脚本会自动通过 `tauri build --config` 注入这些图标，`tauri.conf.json` 保持 `bundle.icon: []` 不变。
`scripts/gen-icon.cjs` 仅在文件不存在时生成占位图，不会覆盖现有图标。
