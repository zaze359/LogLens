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
#   --clean            打包前清空本次平台输出目录（须位于项目 release/ 下，只删该目录内文件）
#   --no-icons         即使图标齐全也不注入 bundle.icon
#   --identity <name>  指定 macOS 签名身份（默认自动挑：Apple 证书 > 本机自签 > ad-hoc）
#   --no-identity      跳过签名，只保留 tauri 的 linker 签名
#                      （环境变量 SIGN_KEYCHAIN=<path> 可指定签名私钥所在的钥匙串，CI 用）
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
#
# macOS 签名：本机用 scripts/make-signers.sh 生成过「LogLens Development」证书时，
# app 与安装脚本都会用它签（完整信任链，重装后权限授权不丢）；没有就回退 ad-hoc。

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
WANT_IDENTITY=""
NO_IDENTITY=0

if [ -t 1 ]; then
  GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RED=$'\033[31m'; DIM=$'\033[2m'; BOLD=$'\033[1m'; RESET=$'\033[0m'
else
  GREEN=""; YELLOW=""; RED=""; DIM=""; BOLD=""; RESET=""
fi

usage() {
  cat <<'EOF'
LogLens 客户端打包脚本（macOS / Linux）

用法: bash scripts/package.sh [选项]

  --out <dir>        输出根目录（默认 <项目根>/release）
  --bundles <list>   tauri build --bundles 的值（默认 mac: app,dmg；linux: deb,appimage）
  --platform <name>  覆盖平台目录名（默认按系统自动检测：macos / linux）
  --skip-build       不重新编译，只收集 target/release/bundle 里已有的产物
  --no-install       跳过 npm install
  --clean            打包前清空本次平台输出目录（只删该平台目录内的文件）
  --no-icons         即使图标齐全也不注入 bundle.icon
  --identity <name>  指定 macOS 签名身份（默认自动挑：Apple 证书 > 本机自签 > ad-hoc）
  --no-identity      跳过签名，只保留 tauri 的 linker 签名
  --open             完成后在 Finder / 文件管理器中打开输出目录
  -h, --help         显示本帮助

产物结构:
  release/macos/LogLens.app                 Apple 应用包（双击运行）
  release/macos/LogLens_0.1.0_aarch64.dmg   安装镜像
  release/macos/LogLens_0.1.0_aarch64.zip   便携包（zip 分发 .app）
  release/<平台>/BUILD-INFO.md              版本 / 平台 / 提交 / 工具链 / 签名状态
  release/<平台>/SHA256SUMS.txt             校验和

设计原则：编译失败就失败，但「打包某一种格式失败」不阻断交付——
例如无权限挂载磁盘卷时 DMG 会失败，此时仍产出 .app + zip 并明确告警。
EOF
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
    --identity)
      shift
      if [ $# -eq 0 ]; then echo "缺少 --identity 的参数"; exit 3; fi
      WANT_IDENTITY="$1"
      ;;
    --no-identity) NO_IDENTITY=1 ;;
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

# 从 GUI / IDE 启动的终端、以及某些 npm 脚本环境不会加载 ~/.cargo/env，
# 导致 rustup 装的 cargo 不在 PATH；先尝试常见安装位置，避免误判“未安装 Rust”。
if ! command -v cargo >/dev/null 2>&1; then
  for cargo_dir in "${CARGO_HOME:-$HOME/.cargo}/bin" /opt/homebrew/bin /usr/local/bin; do
    if [ -x "$cargo_dir/cargo" ]; then
      PATH="$cargo_dir:$PATH"
      export PATH
      info "cargo 不在 PATH，已临时加入 $cargo_dir"
      break
    fi
  done
fi

# 平台名会拼进路径，只允许安全字符；--clean 还会再校验输出目录必须落在 release/ 下
case "$PLATFORM" in
  ''|.*|*..*|*[!A-Za-z0-9._-]*) die "非法平台名: ${PLATFORM}（只允许字母、数字、点、下划线、连字符，且不能以点开头或包含 ..）" ;;
esac

PRODUCT="$(json_get "$ROOT/src-tauri/tauri.conf.json" "productName")"
VERSION="$(json_get "$ROOT/src-tauri/tauri.conf.json" "version")"
PKG_VERSION="$(json_get "$ROOT/package.json" "version")"
if [ "$PKG_VERSION" != "$VERSION" ]; then
  warn "版本号不一致：tauri.conf.json=${VERSION}，package.json=${PKG_VERSION}"
fi
CARGO_VERSION="$(cargo --version 2>/dev/null | awk '{print $2}' || true)"
info "产品 ${PRODUCT} ${VERSION}，目标平台 ${PLATFORM} (${ARCH_TAG})，bundles: ${BUNDLES}"

if [ "$SKIP_BUILD" -eq 0 ]; then
  command -v cargo >/dev/null 2>&1 || die "未找到 cargo（已尝试 \$HOME/.cargo/bin）。请先执行 source \"\$HOME/.cargo/env\"，或参考 README 第 3 节安装 Rust 工具链"
  info "Rust $CARGO_VERSION"
  if [ ! -d "$ROOT/node_modules" ]; then
    if [ "$NO_INSTALL" -eq 1 ]; then
      die "缺少 node_modules 且指定了 --no-install"
    fi
    # 有锁文件就用 npm ci：可复现，且不会悄悄改 lock
    if [ -f "$ROOT/package-lock.json" ]; then
      info "未检测到 node_modules，执行 npm ci"
      run npm ci || die "npm ci 失败"
    else
      info "未检测到 node_modules，执行 npm install"
      run npm install || die "npm install 失败"
    fi
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
    warn "已指定 --no-icons，本次打包使用 Tauri 通用占位图标"
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
  warn "未找到 icon.icns / icon.ico，本次打包${YELLOW}不含自定义图标${RESET}（用 Tauri 通用占位图标）"
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
  # 逐个格式打：单种格式失败（如沙箱里 hdiutil 无法挂载）不应丢掉其它已产出的包。
  # 默认 --bundles app,dmg，实际执行顺序见下方 BUNDLE_ORDER。
  BUILD_OK=0
  BUNDLE_ITEMS="$(printf '%s' "$BUNDLES" | tr ',' ' ')"
  # `tauri build --bundles dmg` 会顺手删掉它临时生成的 bundle/macos/*.app，
  # 所以 app 这一格式必须排在最后：否则等 dmg 打完，收集阶段就只剩 dmg，
  # .app 和便携 zip 都拿不到（README 里承诺的三个产物只能出两个）。
  BUNDLE_ORDER=""
  for bundle in $BUNDLE_ITEMS; do
    [ "$bundle" = "app" ] || BUNDLE_ORDER="$BUNDLE_ORDER $bundle"
  done
  for bundle in $BUNDLE_ITEMS; do
    [ "$bundle" = "app" ] && BUNDLE_ORDER="$BUNDLE_ORDER app"
  done
  # 顺序沿用 --bundles，只把 app 挪到末尾
  for bundle in $BUNDLE_ORDER; do
    if run_build "$bundle"; then
      BUILD_OK=$((BUILD_OK + 1))
      continue
    fi
    case "$bundle" in
      dmg)
        warn "DMG 打包失败（常见原因：hdiutil 需要挂载磁盘卷，无权限或被沙箱拦截）"
        info "  .app 与 zip 不受影响，继续收集产物"
        ;;
      appimage)
        warn "AppImage 打包失败（常见原因：linuxdeploy / FUSE 不可用），deb 不受影响"
        ;;
      *)
        warn "$bundle 打包失败，继续其它格式"
        ;;
    esac
    info "  可单独重试: npm run tauri build -- --bundles $bundle"
  done
  if [ "$BUILD_OK" -eq 0 ]; then
    die "所有格式都编译失败，未产出任何产物"
  fi
else
  step "跳过编译（--skip-build），直接收集已有产物"
fi

# ---------- 2.5 macOS 收尾：重签名 + 把安装脚本塞进 dmg ----------
#
# 签名的选择（详见 scripts/make-signers.sh 与 README「分发给他人」）：
#
# · app 优先用本机自签证书「LogLens Development」（scripts/make-signers.sh 生成）签名。
#   ad-hoc（codesign -s -）是自签里最弱的一种：Gatekeeper 无法证明二进制未被改动，
#   每次重签后 CDHash 都变，系统把 app 当「另一个应用」，TCC 授权（本地网络、通知…）
#   反复要求确认。换成带根证书的自签证书后签名身份稳定，签名质量与 Apple 免费开发者
#   证书同档（缺的只是 Apple 根背书）。没有证书时回退 ad-hoc，并提示怎么生成。
#
# · 「安装 LogLens.command」（原来叫「首次打开-去隔离.command」）用「LogLens Installer」
#   证书签名，依赖理由完全是另一套：没进 quarantine 的脚本由 Finder 直接执行，
#   签名决定它多快被接受；而进了 quarantine 的脚本由 Gatekeeper 评估——只有 Apple
#   签发的开发者证书才免「右键 → 打开」。任何自签证书在这里都没有特权，所以脚本本身
#   不弹「无法验证开发者」，而是老老实实安装，不依赖用户会右键。
#
# · 根证书（不用私钥，--publish 导出）放进 dmg：接收方机器首次遇到这把证书的 app
#   仍可能弹一次「无法验证开发者」，装上根证书后后续启动就不再提示。
HELPER_SRC="$ROOT/scripts/setup-command.command"
HELPER_NAME="安装 LogLens.command"
CERT_DIR="$ROOT/scripts/certs"
DEV_IDENTITY=""
INSTALLER_IDENTITY=""

# 从钥匙串里挑出最合适的签名身份（返回 keychain 里的 SHA-1 哈希，找不到返回空）。
#
# 为什么用哈希而不是名字：同一个名字可能在钥匙串里存在多份（重建证书后旧的没删干净，
# 或同一台机器上还有一张自签的测试证书），codesign -s "名字" 会直接报
#   ambiguous (matches "X" and "X" in ...)
# 打包失败。SHA-1 哈希在钥匙串里唯一，--sign <哈希> 永远不会歧义。
#
# 选择顺序：
#   1) 显式指定（--identity，或 APPLE_SIGNING_IDENTITY —— tauri 也认这个变量），
#      值可以是哈希，也可以是名字（名字匹配到多份时取第一份并告警）；
#   2) Apple 签发的开发者证书（Apple Development / Developer ID Application）优先；
#   3) 本机自签证书。优先用 scripts/certs/SIGNERS-OUTPUT.txt 里记录的指纹 ——
#      那才是 make-signers.sh 刚生成、并随包发布的那张；其它同名旧证书不要顶上来。
# find-identity 的输出形如 `  1) <40 位哈希> "证书名"`：先取出哈希，再和名字比较
pick_hashes_by_name() {
  security find-identity -v -p codesigning 2>/dev/null |
    awk -v w="$1" -F'"' 'NF >= 2 { h = $1; sub(/^ *[0-9]+) */, "", h); sub(/ *$/, "", h); if ($2 == w) print h }'
}
pick_any_hash() {
  local want="${1:-}" hashes n
  if [ -n "$want" ]; then
    case "$want" in
      *[!0-9A-Fa-f]*) hashes="$(pick_hashes_by_name "$want")" ;;
      *) hashes="$(printf '%s\n' "$want" | tr 'a-f' 'A-F')" ;;
    esac
    n="$(printf '%s\n' "$hashes" | grep -c . || true)"
    if [ "${n:-0}" -gt 1 ]; then
      warn "钥匙串里有 ${n} 个证书叫「${want}」，使用第一份（旧的删掉就不会再歧义）"
    fi
    printf '%s\n' "$hashes" | head -1
    return 0
  fi
  local list hash
  list="$(security find-identity -v -p codesigning 2>/dev/null || true)"
  hash="$(printf '%s\n' "$list" | awk -F'"' 'NF >= 2 && ($2 ~ /^Developer ID Application/ || $2 ~ /^Apple Development/) { h = $1; sub(/^ *[0-9]+) */, "", h); sub(/ *$/, "", h); print h; exit }')"
  if [ -n "$hash" ]; then printf '%s\n' "$hash"; return 0; fi
  if [ -f "$CERT_DIR/SIGNERS-OUTPUT.txt" ]; then
    local name fp
    for name in "LogLens-Dev:LogLens Development" "LogLens-Installer:LogLens Installer"; do
      fp="$(sed -n "s/^${name%%:*}: *//p" "$CERT_DIR/SIGNERS-OUTPUT.txt" | tr -d ' :' | tr 'a-f' 'A-F')"
      [ -n "$fp" ] || continue
      if printf '%s\n' "$list" | grep -q "^ *[0-9]*) ${fp} "; then
        printf '%s\n' "$fp"
        return 0
      fi
    done
  fi
  printf '%s\n' "$list" | awk -F'"' 'NF >= 2 && $2 ~ /^LogLens Development/ { h = $1; sub(/^ *[0-9]+) */, "", h); sub(/ *$/, "", h); print h; exit }'
}

# 哈希 -> 证书显示名（日志里别只打一串十六进制）
hash_to_name() {
  security find-identity -v -p codesigning 2>/dev/null |
    awk -v hh="$1" -F'"' 'NF >= 2 { h = $1; sub(/^ *[0-9]+) */, "", h); sub(/ *$/, "", h); if (toupper(h) == toupper(hh)) { print $2; exit } }'
}

# ---------- 2.6 解析签名身份（macOS） ----------
# 只解析一次：app、卷内 app、安装脚本共用同一份结论，日志里能一眼看出这次到底签没签。
resolve_identities() {
  if [ "$NO_IDENTITY" -eq 1 ]; then
    APP_SIGN_IDENTITY=""
    INSTALLER_IDENTITY=""
    info "已指定 --no-identity，跳过重签名"
    return 0
  fi
  local want="${WANT_IDENTITY:-}"
  [ -n "$want" ] || want="${APPLE_SIGNING_IDENTITY:-}"
  DEV_IDENTITY="$(pick_any_hash "$want")"
  if [ -n "$want" ] && [ -z "$DEV_IDENTITY" ]; then
    warn "指定的签名身份「${want}」不在钥匙串里，改为自动选择"
    DEV_IDENTITY="$(pick_any_hash "")"
  fi
  if [ -n "$DEV_IDENTITY" ]; then
    APP_SIGN_IDENTITY="$DEV_IDENTITY"
    ok "app 签名身份：$(hash_to_name "$DEV_IDENTITY")（${DEV_IDENTITY}）"
  else
    # "-" 是 codesign 的 ad-hoc 身份
    APP_SIGN_IDENTITY="-"
    warn "钥匙串里没有可用签名身份，app 使用 ad-hoc 签名"
    info "  生成稳定的本地证书：bash scripts/make-signers.sh"
  fi
  INSTALLER_IDENTITY="$(pick_hashes_by_name "LogLens Installer" | head -1)"
  if [ -n "$INSTALLER_IDENTITY" ]; then
    ok "安装脚本签名身份：LogLens Installer（${INSTALLER_IDENTITY}）"
  fi
}

# 签名前清掉无关扩展属性：tauri 打的包会带 com.apple.provenance，
# 个别 macOS 版本连代码签名校验都会被它带偏，留着只会在验证时出噪音。
strip_xattrs() { xattr -cr "$1" 2>/dev/null || true; }

resign_app() {
  local app="$1" label="${2:-}" identity sign_desc err
  if ! command -v codesign >/dev/null 2>&1; then
    warn "未找到 codesign（需要 Xcode CLT），跳过重签名"
    return 1
  fi
  strip_xattrs "$app"
  identity="${APP_SIGN_IDENTITY:--}"
  if [ -z "$identity" ]; then
    info "${label}$(basename "$app") 跳过重签名（--no-identity）"
    return 0
  fi
  if [ "$identity" = "-" ]; then
    sign_desc="ad-hoc 重签名（重装后 TCC 授权会重新询问）"
  else
    sign_desc="签名为「${identity}」"
  fi
  # 无头环境（CI / SSH）没有 GUI 授权代理，codesign 找不到私钥；
  # SIGN_KEYCHAIN 让打包脚本显式指定私钥所在的钥匙串。
  local kc_args=()
  [ -n "${SIGN_KEYCHAIN:-}" ] && kc_args=(--keychain "$SIGN_KEYCHAIN")
  if codesign --force --deep --sign "$identity" "${kc_args[@]+"${kc_args[@]}"}" "$app" >/dev/null 2>&1 &&
     codesign --verify --strict "$app" >/dev/null 2>&1; then
    ok "${label}$(basename "$app") 已${sign_desc}"
    return 0
  fi
  warn "${label}$(basename "$app") 签名失败，改用 ad-hoc"
  if codesign --force --deep --sign - "$app" >/dev/null 2>&1 && codesign --verify --strict "$app" >/dev/null 2>&1; then
    info "  已回退 ad-hoc（重装后 TCC 授权会重新询问）"
  else
    err="$(codesign --force --deep --sign - "$app" 2>&1 | head -2 | tr '\n' ' ')"
    warn "  ad-hoc 也失败: ${err:-未知错误}；app 保留 tauri 的原始签名"
  fi
  return 1
}

# 卷内根证书：接收方机器第一次遇到这张证书时 app 仍会提示「无法验证开发者」，
# 卷里带一份由安装脚本导入，后续启动就不再提示。只有证书、没有私钥。
stage_root_cert() {
  local stage="$1"
  case "$APP_SIGN_IDENTITY" in
    ""|"Developer ID Application"*|"Apple Development"*) return 0 ;;
  esac
  if [ -f "$CERT_DIR/dist/LogLens-Setup-Root.crt" ]; then
    cp -f "$CERT_DIR/dist/LogLens-Setup-Root.crt" "$stage/LogLens-Setup-Root.crt"
    [ -f "$CERT_DIR/LOG_LAYOUT_DEV_ROOT_SHA256" ] &&
      cp -f "$CERT_DIR/LOG_LAYOUT_DEV_ROOT_SHA256" "$stage/LogLens-Setup-Root.sha256"
    ok "dmg 内已放入根证书 LogLens-Setup-Root.crt（安装脚本会自动导入）"
  else
    info "本机未导出接收方根证书（bash scripts/make-signers.sh --publish），dmg 内不带根证书"
  fi
}

write_dmg_readme() {
  local dest="$1" git_short
  git_short="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo '-')"
  {
    echo "${PRODUCT} ${VERSION}（macOS / ${ARCH_TAG}）"
    echo "构建于 $(date '+%Y-%m-%d %H:%M') · 提交 $git_short"
    echo
    echo "安装"
    echo "  1) 双击「${HELPER_NAME}」，它会清掉隔离属性、拷一份到「应用程序」并启动；"
    echo "  2) 也可以把 $PRODUCT.app 拖到右边的 Applications（应用程序）文件夹，"
    echo "     然后右键 → 打开（首次只需这样一次）。"
    echo
    echo "首次打开被拦下怎么办"
    echo "  · 提示「无法验证开发者」：点「打开」；或在「系统设置 → 隐私与安全性」底部点「仍要打开」。"
    echo "  · 提示「已损坏，无法打开」：这是经微信 / QQ / 飞书等工具传输时给文件打上的"
    echo "    com.apple.quarantine 隔离标记，文件本身没有损坏。双击「${HELPER_NAME}」即可。"
    echo "  · 装好后仍反复询问本地网络 / 通知权限：说明本次 app 没有用正式证书签名，"
    echo "    重装会换掉身份；用「${HELPER_NAME}」里的签名描述确认一次。"
    echo
    echo "签名与校验"
    echo "  应用没有 Apple 开发者签名与公证，证书是本项目自带的；"
    echo "  app 签名信息： codesign -dv --verbose=2 /Applications/$PRODUCT.app"
    echo "  校验和见另发的 zip 分发包里的 SHA256SUMS.txt，一致即说明镜像完好。"
  } > "$dest"
}

# 用「已重签名的 app + tauri 的 dmg 布局」重建 dmg，并在卷内放入安装脚本与说明
build_dmg_with_helper() {
  local app="$1" layout_dmg="$2" dmg_out="$3"
  local stage mnt name
  name="$(basename "$app" .app)"
  stage="$(mktemp -d "${TMPDIR:-/tmp}/loglens-stage.XXXXXX")" || return 1
  mnt="$(mktemp -d "${TMPDIR:-/tmp}/loglens-mnt.XXXXXX")" || return 1

  # 复用 tauri dmg 的窗口布局：挂载后整份拷贝（.DS_Store、卷图标、Applications 链接都在里面）
  if [ -n "$layout_dmg" ] && [ -f "$layout_dmg" ] &&
     hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$layout_dmg" >/dev/null 2>&1; then
    ditto "$mnt/" "$stage/" >/dev/null 2>&1 || cp -R "$mnt"/. "$stage"/ 2>/dev/null || true
    hdiutil detach "$mnt" >/dev/null 2>&1 || true
    # 卷里那份 .app 是 tauri 打的副本（签名不合格），删掉换成重签过的版本
    rm -rf "$stage"/*.app
  fi

  # 必须丢掉 tauri 的 .DS_Store：它只给 LogLens.app 和 Applications 记了图标坐标和窗口大小，
  # 新增的脚本 / 说明没有坐标，Finder 会把它们排到窗口可视区之外（要滚动才看得到），
  # 接收方看起来就像「卷里没有这个文件」。删掉后 Finder 自动排列，所有条目一眼可见。
  rm -f "$stage/.DS_Store"

  if ! ditto "$app" "$stage/$name.app" >/dev/null 2>&1; then
    warn "拷贝 $name.app 到 dmg 暂存区失败"
    rm -rf "$stage" "$mnt"
    return 1
  fi
  [ -e "$stage/Applications" ] || ln -s /Applications "$stage/Applications"

  # 卷内这份 app 也重签：只发 dmg 时，接收方拿到的就是它
  resign_app "$stage/$name.app" "卷内 " || true
  stage_root_cert "$stage"

  if [ -f "$HELPER_SRC" ]; then
    cp -f "$HELPER_SRC" "$stage/$HELPER_NAME" && chmod +x "$stage/$HELPER_NAME" || warn "拷贝安装脚本失败"
    strip_xattrs "$stage/$HELPER_NAME"
    if [ -n "$INSTALLER_IDENTITY" ]; then
      if codesign --force --sign "$INSTALLER_IDENTITY" "$stage/$HELPER_NAME" >/dev/null 2>&1 &&
         codesign --verify --strict "$stage/$HELPER_NAME" >/dev/null 2>&1; then
        ok "卷内安装脚本已签名为「${INSTALLER_IDENTITY}」"
      else
        warn "安装脚本签名失败（未影响打包）"
      fi
    else
      info "安装脚本未签名（双击时 Finder 需要现场决定是否放行；没进 quarantine 时照常执行）"
    fi
  else
    warn "缺少 scripts/setup-command.command：dmg 内不带安装脚本"
  fi
  write_dmg_readme "$stage/使用说明.txt"

  local tmp_dmg="${dmg_out%.dmg}.rebuild.dmg"
  mkdir -p "$(dirname "$dmg_out")"
  rm -f "$tmp_dmg"
  if ! hdiutil create -quiet -volname "${PRODUCT} ${VERSION}" -srcfolder "$stage" -format UDZO "$tmp_dmg" >/dev/null 2>&1; then
    warn "重建 dmg 失败（hdiutil 需要挂载磁盘卷，无权限 / 沙箱环境会被拦），保留原有 dmg"
    rm -rf "$stage" "$mnt" "$tmp_dmg"
    return 1
  fi
  mv -f "$tmp_dmg" "$dmg_out"
  ok "$(basename "$dmg_out")（含安装脚本与使用说明）"
  rm -rf "$stage" "$mnt"
  return 0
}

# macOS 收尾的主流程（原先在一次大改中丢失了入口：函数都在，但从没被调用，
# 于是 dmg 内既没有安装脚本、app 也没重签）。这里补回来。
if [ "$DETECTED_PLATFORM" = "macos" ]; then
  step "macOS 签名"
  resolve_identities

  APP_PATH=""
  for candidate in "$BUNDLE_DIR"/macos/*.app; do
    [ -e "$candidate" ] && APP_PATH="$candidate" && break
  done
  DMG_PATH=""
  for candidate in "$BUNDLE_DIR"/dmg/*.dmg; do
    [ -e "$candidate" ] && DMG_PATH="$candidate" && break
  done

  if [ -n "$APP_PATH" ] || [ -n "$DMG_PATH" ]; then
    step "macOS 收尾：重签名 + 重建 dmg（内含安装脚本）"
    APP_FOR_DMG="$APP_PATH"
    # 只打了 dmg 的情况（--bundles dmg）：先从卷里把 app 取出来，再统一走重签 + 重建
    if [ -z "$APP_FOR_DMG" ] && [ -n "$DMG_PATH" ]; then
      EXTRACT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/loglens-x.XXXXXX")"
      MNT="$(mktemp -d "${TMPDIR:-/tmp}/loglens-m.XXXXXX")"
      if hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG_PATH" >/dev/null 2>&1; then
        for candidate in "$MNT"/*.app; do
          [ -e "$candidate" ] || continue
          ditto "$candidate" "$EXTRACT_DIR/$(basename "$candidate")" >/dev/null 2>&1 ||
            cp -R "$candidate" "$EXTRACT_DIR/" 2>/dev/null || true
        done
        hdiutil detach "$MNT" >/dev/null 2>&1 || true
      fi
      APP_FOR_DMG="$(ls -d "$EXTRACT_DIR"/*.app 2>/dev/null | head -1 || true)"
      info "已从 dmg 中取出 app 用于重签：${APP_FOR_DMG:-未取到}"
      rm -rf "$MNT"
    fi

    if [ -n "$APP_PATH" ]; then
      resign_app "$APP_PATH" || true
    fi

    if [ -n "$APP_FOR_DMG" ]; then
      TARGET_DMG="${DMG_PATH:-$BUNDLE_DIR/dmg/${PRODUCT}_${VERSION}_${ARCH_TAG}.dmg}"
      build_dmg_with_helper "$APP_FOR_DMG" "$DMG_PATH" "$TARGET_DMG" || true
    fi
    if [ -n "${EXTRACT_DIR:-}" ]; then rm -rf "$EXTRACT_DIR"; fi
  fi
fi

# ---------- 3. 收集产物 ----------
mkdir -p "$OUT_ROOT"
OUT="$OUT_ROOT/$PLATFORM"

# --platform / --out 都会拼进路径，--clean 前必须确认落在 <项目根>/release 之下，
# 否则 `--platform ../..` 会删掉项目根之外的一级内容。
if [ "$DO_CLEAN" -eq 1 ]; then
  ROOT_REAL="$(cd "$ROOT" && pwd -P)"
  OUT_PARENT_REAL="$(cd "$OUT_ROOT" && pwd -P)"
  case "$OUT_PARENT_REAL/" in
    "$ROOT_REAL/release/"*) ;;
    *) die "拒绝在仓库外执行 --clean: ${OUT_PARENT_REAL}（输出根目录必须在 ${ROOT_REAL}/release 下）" ;;
  esac
  if [ -d "$OUT" ]; then
    step "清空 $OUT"
    find "$OUT" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
  fi
fi

mkdir -p "$OUT"

# Finder 在这些目录里活动（打开过 release/ 或解压过产物）会留下 .DS_Store，
# 它既不是产物也不该进校验和，收集时直接清掉。
find "$OUT" -mindepth 1 -maxdepth 1 -name '.DS_Store' -delete 2>/dev/null || true

step "收集产物到 ${OUT#"$ROOT"/}/"
COLLECTED=0

# bundle/ 不会自动清掉历史版本，而 dmg / deb / AppImage 的文件名带版本号：
# 只收当前版本的产物，避免把 0.0.x 的包当成 0.1.0 一起发出去。
collect_versioned() {
  local f
  for f in "$@"; do
    [ -e "$f" ] || continue
    case "$(basename "$f")" in
      *"$VERSION"*)
        cp -f "$f" "$OUT/"
        ok "$(basename "$f")"
        COLLECTED=$((COLLECTED + 1))
        ;;
      *)
        warn "跳过旧版本产物 $(basename "$f")（当前版本 ${VERSION}）"
        ;;
    esac
  done
}

copy_dmg() {
  collect_versioned "$BUNDLE_DIR"/dmg/*.dmg
}

copy_linux_pkgs() {
  collect_versioned "$BUNDLE_DIR"/deb/*.deb "$BUNDLE_DIR"/appimage/*.AppImage "$BUNDLE_DIR"/rpm/*.rpm
}

copy_app() {
  local src name
  for src in "$BUNDLE_DIR"/macos/*.app; do
    [ -e "$src" ] || continue
    name="$(basename "$src")"
    rm -rf "${OUT:?}/$name"
    # 优先 ditto：复制时保留扩展属性 / 资源分叉（签名信息），cp -R 会丢
    if command -v ditto >/dev/null 2>&1; then
      ditto "$src" "$OUT/$name"
    else
      cp -R "$src" "$OUT/"
    fi
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
  collect_versioned "$BUNDLE_DIR"/deb/*.deb "$BUNDLE_DIR"/appimage/*.AppImage "$BUNDLE_DIR"/rpm/*.rpm
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

# IM（微信/QQ/飞书）传输会给未签名 app 打上 quarantine，Gatekeeper 报「已损坏」。
# 附带安装脚本：接收方双击即可把 app 拷进「应用程序」并清掉隔离属性，不必敲命令。
# 脚本的签名与 dmg 卷内那份一致，两个入口不会出现「一个能用一个不能用」。
if [ "$DETECTED_PLATFORM" = "macos" ]; then
  if [ -f "$HELPER_SRC" ]; then
    HELPER="$OUT/$HELPER_NAME"
    cp -f "$HELPER_SRC" "$HELPER" && chmod +x "$HELPER"
    strip_xattrs "$HELPER"
    if [ -n "${INSTALLER_IDENTITY:-}" ]; then
      codesign --force --sign "$INSTALLER_IDENTITY" "$HELPER" >/dev/null 2>&1 || true
    fi
    ok "$HELPER_NAME"
  else
    warn "缺少 scripts/setup-command.command，产物不含安装脚本"
  fi
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
ICON_SUFFIX=""
if [ "${ICON_ARGS[*]-}" != "" ]; then ICON_SUFFIX=" + bundle.icon"; fi
if [ "$SKIP_BUILD" -eq 1 ]; then
  BUILD_DESC="跳过编译（--skip-build），仅收集已有产物"
else
  # 这里只记录命令文本；早期版本误用了反引号，会在写构建信息时又跑一遍
  # 不带 bundle.icon 的 tauri build，Linux 上 AppImage 因找不到方形图标直接 abort。
  BUILD_DESC="npm run tauri build -- --bundles $BUNDLES$ICON_SUFFIX"
fi
case "$PLATFORM" in
  macos)
    if [ "${NO_IDENTITY:-0}" -eq 1 ]; then
      SIGNING_DESC="未重签名（--no-identity），只有 tauri 的 linker 级 ad-hoc 签名；Gatekeeper 可能判「已损坏」"
    elif [ "${APP_SIGN_IDENTITY:--}" = "-" ]; then
      SIGNING_DESC="ad-hoc 签名（未公证）：每次重装系统都当新应用，TCC 授权会重新询问；建议先跑 scripts/make-signers.sh"
    else
      SIGNING_DESC="用本机证书「${APP_SIGN_IDENTITY}」签名（未公证）；首次打开可能提示「无法验证开发者」，右键 → 打开一次即可"
    fi
    ;;
  windows) SIGNING_DESC="未签名（SmartScreen 可能提示「未知发布者」）" ;;
  *) SIGNING_DESC="未签名（未做 GPG 签名）" ;;
esac
{
  echo "# $PRODUCT 打包信息"
  echo
  echo "| 项目 | 值 |"
  echo "| --- | --- |"
  echo "| 产品 / 版本 | $PRODUCT $VERSION |"
  echo "| 平台 / 架构 | $PLATFORM / $ARCH_TAG |"
  echo "| 打包时间 | $(date '+%Y-%m-%d %H:%M:%S %z') |"
  echo "| Git | $GIT_BRANCH@$GIT_COMMIT $GIT_DIRTY|"
  echo "| 打包方式 | $BUILD_DESC |"
  echo "| 签名 | $SIGNING_DESC |"
  echo "| 工具链 | Node $(node -v) / Rust ${CARGO_VERSION:-未知} / Tauri CLI ${TAURI_CLI:-未知} |"
  echo
  echo "## 文件"
  echo
  echo "| 文件 | 大小 |"
  echo "| --- | --- |"
} > "$OUT/BUILD-INFO.md"

# du -s 才是一行一个条目；不加会在 .app 里逐层递归。隐藏文件（.DS_Store 等）不列
(
  cd "$OUT" || exit 1
  find . -mindepth 1 -maxdepth 1 ! -name '.*' ! -name 'BUILD-INFO.md' ! -name 'SHA256SUMS.txt' \
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
  elif command -v sha256sum >/dev/null 2>&1; then
    HASHER="sha256sum"
  else
    die "需要 shasum 或 sha256sum 才能生成校验和"
  fi
  # shellcheck disable=SC2086
  # ! -name '.*' 与上面 BUILD-INFO 的文件清单保持一致：.DS_Store 之类的点文件不算产物
  find . -mindepth 1 -maxdepth 1 -type f ! -name '.*' ! -name 'SHA256SUMS.txt' ! -name 'BUILD-INFO.md' \
    -exec $HASHER {} + | sed 's| \./| |' | sort -k2 > SHA256SUMS.txt
)
ok "SHA256SUMS.txt"

# ---------- 5. 汇总 ----------
step "完成"
echo "  输出目录: ${BOLD}${OUT#"$ROOT"/}${RESET}  ($(du -sh "$OUT" | awk '{print $1}'))"
find "$OUT" -mindepth 1 -maxdepth 1 -exec basename {} \; | sort | sed 's/^/    /'

if [ "$PLATFORM" = "macos" ] && [ "$SKIP_BUILD" -eq 0 ] && [ "${ICON_ARGS[*]-}" = "" ]; then
  echo
  warn "本次产物未带自定义应用图标（bundle.icon 为空），正式发布前请生成图标后重跑本脚本"
fi

if [ "$DETECTED_PLATFORM" = "macos" ] && [ "${NO_IDENTITY:-0}" -eq 0 ] && [ "$SKIP_BUILD" -eq 0 ]; then
  if [ "${APP_SIGN_IDENTITY:--}" = "-" ]; then
    echo
    warn "app 是 ad-hoc 签名：每次重装都会被当成新应用。"
    info "  先执行 bash scripts/make-signers.sh 生成证书，再跑一次 bash scripts/package.sh --skip-build 即可重签。"
  fi
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
