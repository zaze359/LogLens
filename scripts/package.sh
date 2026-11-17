#!/usr/bin/env bash
# LogLens 客户端打包脚本（macOS / Linux）
#
# 一条命令产出可分发的客户端，并把所有产物统一收集到项目根目录下的
# release/<平台>/，不再需要去 src-tauri/target/release/bundle/ 里翻找。
#
# 用法: bash scripts/package.sh [选项]
#
#   --out <dir>        输出根目录（默认 <项目根>/release）
#   --bundles <list>   tauri build --bundles 的值（默认 mac: app,dmg；linux: deb,appimage）
#   --platform <name>  覆盖平台目录名（默认按系统自动检测：macos / linux）
#   --skip-build       不重新编译，只收集 target/release/bundle 里已有的产物
#   --no-install       跳过 npm install
#   --clean            打包前清空本次平台输出目录（只删该平台目录内的文件）
#   --no-icons         即使图标齐全也不注入 bundle.icon
#   --open             完成后在 Finder / 文件管理器中打开输出目录
#   -h, --help         显示本帮助
#
# 产物结构:
#   release/macos/LogLens.app              Apple 应用包（双击运行）
#   release/macos/LogLens_0.1.0_aarch64.dmg  安装镜像（需 hdiutil 可用）
#   release/macos/LogLens_0.1.0_aarch64.zip  便携包（zip 分发 .app）
#   release/macos/BUILD-INFO.md            版本 / 平台 / 提交 / 工具链
#   release/macos/SHA256SUMS.txt           校验和
#
# 设计原则：编译失败就失败，但「打包某一种格式失败」不阻断交付——
# 例如无权限挂载磁盘卷时 DMG 会失败，此时仍产出 .app + zip 并明确告警。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

OUT_ROOT="$ROOT/release"
PLATFORM_OVERRIDE=""
BUNDLES=""
SKIP_BUILD=0
NO_INSTALL=0
DO_CLEAN=0
NO_ICONS=0
DO_OPEN=0

if [ -t 1 ]; then
  GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RED=$'\033[31m'; DIM=$'\033[2m'; BOLD=$'\033[1m'; RESET=$'\033[0m'
else
  GREEN=""; YELLOW=""; RED=""; DIM=""; BOLD=""; RESET=""
fi

usage() {
  sed -n '2,27p' "${BASH_SOURCE[0]}" | sed -E 's/^# ?//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out)
      shift
      if [ $# -eq 0 ]; then echo "缺少 --out 的目录参数"; exit 3; fi
      OUT_ROOT="$1"
      ;;
    --bundles)
      shift
      if [ $# -eq 0 ]; then echo "缺少 --bundles 的参数"; exit 3; fi
      BUNDLES="$1"
      ;;
    --platform)
      shift
      if [ $# -eq 0 ]; then echo "缺少 --platform 的参数"; exit 3; fi
      PLATFORM_OVERRIDE="$1"
      ;;
    --skip-build) SKIP_BUILD=1 ;;
    --no-install) NO_INSTALL=1 ;;
    --clean) DO_CLEAN=1 ;;
    --no-icons) NO_ICONS=1 ;;
    --open) DO_OPEN=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数: $1"; echo; usage; exit 3 ;;
  esac
  shift
done

# ---------- helpers ----------
step() { echo; echo "${BOLD}── $* ──${RESET}"; }
info() { echo "  $*"; }
ok()   { echo "  ${GREEN}✓${RESET} $*"; }
warn() { echo "  ${YELLOW}!${RESET} $*"; }
die()  { echo "  ${RED}✗${RESET} $*" >&2; exit 2; }
run()  { echo "  ${DIM}\$ $*${RESET}"; "$@"; }

# 平台 / 架构归一化，和 Tauri 自己的产物命名保持一致
detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin) DETECTED_PLATFORM="macos" ;;
    Linux)  DETECTED_PLATFORM="linux" ;;
    *)
      echo "当前系统 ($os) 请使用 scripts/package.ps1（Windows）或手动打包。" >&2
      exit 2
      ;;
  esac
  case "$arch" in
    arm64|aarch64) ARCH_TAG="aarch64" ;;
    x86_64|amd64)  ARCH_TAG="x64" ;;
    *)             ARCH_TAG="$arch" ;;
  esac
}

json_get() {
  # json_get <file> <key>
  node -e 'const fs=require("fs");const c=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));console.log(c[process.argv[2]])' "$1" "$2"
}

# ---------- 0. 环境预检 ----------
detect_platform
PLATFORM="${PLATFORM_OVERRIDE:-$DETECTED_PLATFORM}"

if [ -z "$BUNDLES" ]; then
  case "$PLATFORM" in
    macos) BUNDLES="app,dmg" ;;
    linux) BUNDLES="deb,appimage" ;;
    *)     BUNDLES="app" ;;
  esac
fi

step "环境检查"
command -v node >/dev/null 2>&1 || die "未找到 node，请先安装 Node.js >= 18"
command -v npm  >/dev/null 2>&1 || die "未找到 npm"
info "Node $(node -v) / npm $(npm -v)"

PRODUCT="$(json_get "$ROOT/src-tauri/tauri.conf.json" "productName")"
VERSION="$(json_get "$ROOT/src-tauri/tauri.conf.json" "version")"
info "产品 ${PRODUCT} ${VERSION}，目标平台 ${PLATFORM} (${ARCH_TAG})，bundles: ${BUNDLES}"

if [ "$SKIP_BUILD" -eq 0 ]; then
  command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust 工具链（见 README 第 3 节）"
  info "Rust $(cargo --version | awk '{print $2}')"
  if [ ! -d "$ROOT/node_modules" ]; then
    if [ "$NO_INSTALL" -eq 1 ]; then
      die "缺少 node_modules 且指定了 --no-install"
    fi
    info "未检测到 node_modules，执行 npm install"
    run npm install || die "npm install 失败"
  fi
  # generate_context!() 编译期会读 icons/icon.png，缺了会直接编译失败
  run node "$ROOT/scripts/gen-icon.cjs" >/dev/null || die "生成开发态窗口图标失败"
fi

# ---------- 1. 打包图标 ----------
# 不写回 tauri.conf.json：用 --config 覆盖，避免污染受版本控制的配置。
ICON_ARGS=()
ensure_icons() {
  local icons="$ROOT/src-tauri/icons" src
  if [ "$NO_ICONS" -eq 1 ]; then
    warn "已指定 --no-icons，本次打包使用系统默认图标"
    return 0
  fi
  if [ -f "$icons/icon.icns" ] && [ -f "$icons/icon.ico" ]; then
    ICON_ARGS=(--config '{"bundle":{"icon":["icons/32x32.png","icons/128x128.png","icons/128x128@2x.png","icons/icon.icns","icons/icon.ico"]}}')
    ok "图标齐全，已注入 bundle.icon（不修改 tauri.conf.json）"
    return 0
  fi
  # 允许「往根目录丢一张 1024x1024 的 app-icon.png」自动生成整套图标
  for src in "$ROOT/app-icon.png" "$ROOT/src-tauri/app-icon.png"; do
    if [ -f "$src" ]; then
      info "发现 $(basename "$(dirname "$src")")/$(basename "$src")，自动生成图标集"
      if run npm run tauri icon -- "$src" >/dev/null 2>&1 && [ -f "$icons/icon.icns" ] && [ -f "$icons/icon.ico" ]; then
        ICON_ARGS=(--config '{"bundle":{"icon":["icons/32x32.png","icons/128x128.png","icons/128x128@2x.png","icons/icon.icns","icons/icon.ico"]}}')
        ok "图标已生成并注入"
        return 0
      fi
      warn "从 $src 生成图标失败，继续使用默认图标"
      break
    fi
  done
  warn "未找到 icon.icns / icon.ico，本次打包${YELLOW}不含自定义图标${RESET}（用系统默认图标）"
  info "生成方式: 准备 1024x1024 PNG 后执行  npm run tauri icon <png>"
  return 0
}

# ---------- 2. 编译 ----------
BUNDLE_DIR="$ROOT/src-tauri/target/release/bundle"

run_build() {
  local bundles="$1"
  echo "  ${DIM}\$ npm run tauri build -- --bundles $bundles${RESET}"
  # ${arr[@]+...} 保证 bash 3.2 + set -u 下空数组安全展开
  npm run tauri build -- --bundles "$bundles" ${ICON_ARGS[@]+"${ICON_ARGS[@]}"}
}

if [ "$SKIP_BUILD" -eq 0 ]; then
  ensure_icons
  step "编译客户端（${PRODUCT} ${VERSION}）"
  # DMG 单独打：它依赖 hdiutil 挂载磁盘卷，在无权限 / 沙箱环境下会失败，
  # 但 .app 本身完全可用，不应该因为这一步整体失败。
  PRIMARY_BUNDLES="$(printf '%s' "$BUNDLES" | tr ',' '\n' | grep -v '^dmg$' | paste -sd, - || true)"
  if [ -z "$PRIMARY_BUNDLES" ]; then PRIMARY_BUNDLES="app"; fi
  run_build "$PRIMARY_BUNDLES" || die "编译失败，未产出任何产物"

  case ",$BUNDLES," in
    *,dmg,*)
      step "打包 DMG 安装镜像"
      if run_build "dmg"; then
        ok "DMG 生成成功"
      else
        warn "DMG 打包失败（常见原因：hdiutil 需要挂载磁盘卷，无权限或被沙箱拦截）"
        info "  可在本地终端重试: npm run tauri build -- --bundles dmg"
        info "  .app 与 zip 不受影响，继续收集产物"
      fi
      ;;
  esac
else
  step "跳过编译（--skip-build），直接收集已有产物"
fi

# ---------- 3. 收集产物 ----------
OUT="$OUT_ROOT/$PLATFORM"

# 清理范围限制在本次平台目录内，避免误删别的平台的产物
if [ "$DO_CLEAN" -eq 1 ] && [ -d "$OUT" ]; then
  step "清空 $OUT"
  find "$OUT" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
fi

mkdir -p "$OUT"

step "收集产物到 ${OUT#"$ROOT"/}/"
COLLECTED=0

copy_dmg() {
  local f
  for f in "$BUNDLE_DIR"/dmg/*.dmg; do
    [ -e "$f" ] || continue
    cp -f "$f" "$OUT/"
    ok "$(basename "$f")"
    COLLECTED=$((COLLECTED + 1))
  done
}

copy_app() {
  local src name
  for src in "$BUNDLE_DIR"/macos/*.app; do
    [ -e "$src" ] || continue
    name="$(basename "$src")"
    rm -rf "${OUT:?}/$name"
    cp -R "$src" "$OUT/"
    ok "$name"
    COLLECTED=$((COLLECTED + 1))
  done
}

zip_app() {
  local src name base
  for src in "$BUNDLE_DIR"/macos/*.app; do
    [ -e "$src" ] || continue
    name="$(basename "$src" .app)"
    base="${name}_${VERSION}_${ARCH_TAG}.zip"
    rm -f "$OUT/$base"
    # ditto 保留资源分叉 / 签名信息，比 zip -r 更可靠
    if ditto -c -k --sequesterRsrc --keepParent "$src" "$OUT/$base"; then
      ok "$base"
      COLLECTED=$((COLLECTED + 1))
    else
      warn "生成 $base 失败"
    fi
  done
}

copy_linux_pkgs() {
  local f
  for f in "$BUNDLE_DIR"/deb/*.deb "$BUNDLE_DIR"/appimage/*.AppImage "$BUNDLE_DIR"/rpm/*.rpm; do
    [ -e "$f" ] || continue
    cp -f "$f" "$OUT/"
    ok "$(basename "$f")"
    COLLECTED=$((COLLECTED + 1))
  done
}

# 收集策略跟着「实际系统」走，--platform 只改输出目录名，避免把 mac 的 .app 收进 windows 目录
case "$DETECTED_PLATFORM" in
  macos)
    copy_app
    zip_app
    copy_dmg
    ;;
  linux)
    copy_linux_pkgs
    ;;
esac

if [ "$COLLECTED" -eq 0 ]; then
  die "没有收集到任何产物，请检查 $BUNDLE_DIR"
fi

# ---------- 4. 构建信息 + 校验和 ----------
GIT_BRANCH="$(git -C "$ROOT" branch --show-current 2>/dev/null || echo '-')"
GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo '-')"
if [ -n "$(git -C "$ROOT" status --porcelain 2>/dev/null || true)" ]; then
  GIT_DIRTY="（工作区有未提交改动）"
else
  GIT_DIRTY=""
fi
TAURI_CLI="$(npx --no-install tauri --version 2>/dev/null | awk '{print $2}' || echo '-')"

step "写入构建信息"
{
  echo "# $PRODUCT 打包信息"
  echo
  echo "| 项目 | 值 |"
  echo "| --- | --- |"
  echo "| 产品 / 版本 | $PRODUCT $VERSION |"
  echo "| 平台 / 架构 | $PLATFORM / $ARCH_TAG |"
  echo "| 打包时间 | $(date '+%Y-%m-%d %H:%M:%S %z') |"
  echo "| Git | $GIT_BRANCH@$GIT_COMMIT $GIT_DIRTY|"
  echo "| 打包方式 | \`npm run tauri build -- --bundles $BUNDLES\`${ICON_ARGS:+ + bundle.icon} |"
  echo "| 工具链 | Node $(node -v) / $(cargo --version 2>/dev/null | awk '{print "Rust "$2}') / Tauri CLI ${TAURI_CLI:-未知} |"
  echo
  echo "## 文件"
  echo
  echo "| 文件 | 大小 |"
  echo "| --- | --- |"
} > "$OUT/BUILD-INFO.md"

# du -s 才是一行一个条目；不加会在 .app 里逐层递归
(
  cd "$OUT" || exit 1
  find . -mindepth 1 -maxdepth 1 ! -name 'BUILD-INFO.md' ! -name 'SHA256SUMS.txt' \
    -exec du -hs {} + 2>/dev/null | sort -k2 |
    awk -F'\t' '{sub(/^\.\//, "", $2); printf "| `%s` | %s |\n", $2, $1}'
) >> "$OUT/BUILD-INFO.md"

{
  echo
  echo "> \`SHA256SUMS.txt\` 覆盖所有文件类产物；\`.app\` 是目录，不参与校验（分发给他人时用 zip 包）。"
} >> "$OUT/BUILD-INFO.md"
ok "BUILD-INFO.md"

step "生成校验和"
(
  cd "$OUT" || exit 1
  if command -v shasum >/dev/null 2>&1; then
    HASHER="shasum -a 256"
  else
    HASHER="sha256sum"
  fi
  # shellcheck disable=SC2086
  find . -mindepth 1 -maxdepth 1 -type f ! -name 'SHA256SUMS.txt' ! -name 'BUILD-INFO.md' \
    -exec $HASHER {} + | sed 's| \./| |' | sort -k2 > SHA256SUMS.txt
)
ok "SHA256SUMS.txt"

# ---------- 5. 汇总 ----------
step "完成"
echo "  输出目录: ${BOLD}${OUT#"$ROOT"/}${RESET}  ($(du -sh "$OUT" | awk '{print $1}'))"
find "$OUT" -mindepth 1 -maxdepth 1 -exec basename {} \; | sort | sed 's/^/    /'

if [ "$PLATFORM" = "macos" ] && [ "${#ICON_ARGS[@]}" -eq 0 ]; then
  echo
  warn "本次产物未带自定义应用图标（bundle.icon 为空），正式发布前请生成图标后重跑本脚本"
fi

if [ "$DO_OPEN" -eq 1 ]; then
  if [ "$(uname -s)" = "Darwin" ]; then
    open "$OUT"
  elif command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$OUT" >/dev/null 2>&1 || true
  fi
fi

# 显式成功：CI 里判断「有没有出包」只看退出码，别让尾部可选步骤的返回码干扰
exit 0
