#!/usr/bin/env bash
# LogLens 一键环境检测 / 安装 / 运行（macOS；Linux 可检测 + 跑项目，但不自动装系统依赖）
# 用法: bash scripts/setup.sh [--check-only] [--no-system-install] [--build|--test|--dev]
#                            [--no-run] [--report <path>] [--help]
#
# 设计原则：可重复执行（幂等）、缺失才安装、每一步都打印实际执行的命令。
# 系统级安装（Homebrew / rustup / Xcode CLT）只在本脚本中执行；不想让脚本改动系统，
# 用 --no-system-install 或 --check-only。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MIN_NODE="18.0.0"
MIN_RUST="1.90.0"   # Tauri 2 官方 dialog 插件要求 Rust >= 1.90
MODE="dev"
CHECK_ONLY=0
NO_SYSTEM_INSTALL=0
NO_RUN=0
REPORT=""
CLT_PENDING=0

if [ -t 1 ]; then
  GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RED=$'\033[31m'; RESET=$'\033[0m'
else
  GREEN=""; YELLOW=""; RED=""; RESET=""
fi

usage() {
  cat <<'EOF'
LogLens 一键环境检测 / 安装 / 运行

  bash scripts/setup.sh [选项]

选项:
  (无参数)              检测 -> 自动安装缺失项 -> npm install -> 启动客户端
  --check-only          只输出环境检测报告，不安装、不运行（有缺失时退出码 2）
  --no-system-install   不安装系统级依赖（Node / Rust / Homebrew / CLT），仍装项目依赖
  --no-run              装好依赖但不启动 / 不构建
  --dev                 默认；安装后运行 npm run tauri dev
  --build               安装后运行 npm run tauri build（检查图标并自动补全 bundle.icon）
  --test                安装后运行 npm run build + cargo test --lib
  --report <path>       额外输出 Markdown 报告到指定文件
  -h, --help            显示本帮助

示例:
  bash scripts/setup.sh
  bash scripts/setup.sh --check-only --report ./setup-report.md
  bash scripts/setup.sh --no-system-install --no-run
  bash scripts/setup.sh --build
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --check-only) CHECK_ONLY=1 ;;
    --no-system-install) NO_SYSTEM_INSTALL=1 ;;
    --no-run) NO_RUN=1 ;;
    --dev) MODE="dev" ;;
    --build) MODE="build" ;;
    --test) MODE="test" ;;
    --report)
      shift
      if [ $# -eq 0 ]; then echo "缺少 --report 的路径参数"; exit 3; fi
      REPORT="$1"
      ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数: $1"; usage; exit 3 ;;
  esac
  shift
done

# An already-installed rustup toolchain can be invisible when the caller's shell never
# sourced $HOME/.cargo/env. Load it before detection so we don't re-run the installer.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

# ---------- helpers ----------
version_ge() {
  # awk 逐段比较，避开 macOS BSD sort 没有 -V 的问题
  awk -v a="$1" -v b="$2" 'BEGIN {
    n = split(a, A, "."); m = split(b, B, ".");
    k = (n > m) ? n : m;
    for (i = 1; i <= k; i++) {
      x = (i <= n ? A[i] : 0) + 0;
      y = (i <= m ? B[i] : 0) + 0;
      if (x > y) exit 0;
      if (x < y) exit 1;
    }
    exit 0;
  }'
}

clean_version() {
  printf '%s' "$1" | sed -E 's/^v//' | sed -E 's/[^0-9.].*$//'
}

RESULTS=()
HAS_FAIL=0
record() {
  local status="$1" name="$2" current="$3" require="$4"
  RESULTS+=("$status|$name|$current|$require")
  if [ "$status" = "FAIL" ]; then HAS_FAIL=1; fi
}

# ${arr[@]+"${arr[@]}"} keeps bash 3.2 + `set -u` happy when the array is empty.
print_table() {
  printf "%-9s %-18s %-34s %-24s\n" "状态" "项目" "当前" "要求"
  printf "%-9s %-18s %-34s %-24s\n" "--------" "----------------" "--------------------------------" "-----------------------"
  local row s n c r color
  for row in ${RESULTS[@]+"${RESULTS[@]}"}; do
    IFS='|' read -r s n c r <<< "$row"
    color=""
    case "$s" in
      OK) color="$GREEN" ;;
      WARN) color="$YELLOW" ;;
      FAIL) color="$RED" ;;
    esac
    printf "%s%-9s%s %-18s %-34s %-24s\n" "$color" "$s" "$RESET" "$n" "$c" "$r"
  done
}

write_report() {
  # Note: `if` rather than `[ ... ] && return`, which would trip `set -e`.
  if [ -z "$REPORT" ]; then return 0; fi
  local row s n c r parent
  parent="$(dirname "$REPORT")"
  if [ ! -d "$parent" ]; then mkdir -p "$parent"; fi
  {
    echo "# LogLens 环境检测报告"
    echo
    echo "- 时间: $(date '+%Y-%m-%d %H:%M:%S')"
    echo "- 系统: $(uname -s) $(uname -m)"
    echo "- 模式: $MODE（check-only=$CHECK_ONLY）"
    echo
    echo "| 状态 | 项目 | 当前 | 要求 |"
    echo "| --- | --- | --- | --- |"
    for row in ${RESULTS[@]+"${RESULTS[@]}"}; do
      IFS='|' read -r s n c r <<< "$row"
      echo "| $s | $n | $c | $r |"
    done
  } > "$REPORT"
  echo "报告已写入: $REPORT"
}

run_cmd() {
  echo "  \$ $*"
  if "$@"; then
    return 0
  fi
  echo "  ! 命令失败: $*"
  return 1
}

# ---------- detection ----------
detect() {
  RESULTS=()
  HAS_FAIL=0
  local os pid clt brew_v rustc_v cargo_v node_v

  os="$(uname -s)"
  record OK "操作系统" "$os / $(uname -m)" "macOS（Windows 用 setup.ps1）"

  if command -v node >/dev/null 2>&1; then
    node_v="$(clean_version "$(node -v 2>/dev/null)")"
    if version_ge "$node_v" "$MIN_NODE"; then
      record OK "Node.js" "v$node_v" ">= $MIN_NODE"
    else
      record FAIL "Node.js" "v$node_v" ">= $MIN_NODE"
    fi
  else
    record FAIL "Node.js" "未安装" ">= $MIN_NODE"
  fi

  if command -v npm >/dev/null 2>&1; then
    record OK "npm" "$(npm -v 2>/dev/null)" "随 Node 提供"
  else
    record FAIL "npm" "未安装" "随 Node 提供"
  fi

  if command -v rustc >/dev/null 2>&1; then
    rustc_v="$(rustc --version 2>/dev/null | awk '{print $2}')"
    rustc_v="$(clean_version "$rustc_v")"
    if version_ge "$rustc_v" "$MIN_RUST"; then
      record OK "Rust (rustc)" "$rustc_v" ">= $MIN_RUST"
    else
      record FAIL "Rust (rustc)" "$rustc_v" ">= $MIN_RUST"
    fi
  else
    record FAIL "Rust (rustc)" "未安装" ">= $MIN_RUST"
  fi

  if command -v cargo >/dev/null 2>&1; then
    cargo_v="$(cargo --version 2>/dev/null | awk '{print $2}')"
    record OK "Rust (cargo)" "$(clean_version "$cargo_v")" "必须"
  else
    record FAIL "Rust (cargo)" "未安装" "必须"
  fi

  if command -v rustup >/dev/null 2>&1; then
    record OK "rustup" "$(rustup -V 2>/dev/null | awk '{print $2}')" "可选（用于升级 Rust）"
  else
    record WARN "rustup" "未安装" "版本过低时需要"
  fi

  if [ "$os" = "Darwin" ]; then
    if xcode-select -p >/dev/null 2>&1; then
      clt="$(xcode-select -p 2>/dev/null)"
      record OK "Xcode CLT" "$clt" "必须"
    else
      record FAIL "Xcode CLT" "未安装" "必须（会触发系统弹窗）"
    fi
    if command -v brew >/dev/null 2>&1; then
      brew_v="$(brew --version 2>/dev/null | head -n 1 || true)"
      record OK "Homebrew" "$brew_v" "可选（用于安装 Node）"
    else
      record WARN "Homebrew" "未安装" "缺 Node 时自动安装"
    fi
  fi

  if command -v lsof >/dev/null 2>&1; then
    pid="$(lsof -nP -iTCP:1420 -sTCP:LISTEN -t 2>/dev/null | head -n 1 || true)"
    if [ -n "$pid" ]; then
      record WARN "端口 1420" "被 PID $pid 占用" "空闲（Vite 固定端口）"
    else
      record OK "端口 1420" "空闲" "空闲"
    fi
  fi

  if [ -f "$ROOT/src-tauri/icons/icon.png" ]; then
    record OK "开发图标" "icon.png 已存在" "generate_context! 编译必需"
  else
    record WARN "开发图标" "未生成（启动前自动创建）" "generate_context! 编译必需"
  fi

  if [ -f "$ROOT/src-tauri/icons/icon.icns" ] && [ -f "$ROOT/src-tauri/icons/icon.ico" ]; then
    record OK "打包图标" "全套已生成" "打包必需"
  else
    record WARN "打包图标" "未生成" "打包前 npm run tauri icon <png>"
  fi

  if [ -d "$ROOT/node_modules" ]; then
    record OK "前端依赖" "node_modules 已存在" "npm install"
  else
    record WARN "前端依赖" "尚未安装" "npm install"
  fi
}

node_ok() {
  if ! command -v node >/dev/null 2>&1; then return 1; fi
  version_ge "$(clean_version "$(node -v 2>/dev/null)")" "$MIN_NODE"
}

rust_ok() {
  if ! command -v rustc >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then return 1; fi
  version_ge "$(clean_version "$(rustc --version 2>/dev/null | awk '{print $2}')")" "$MIN_RUST"
}

# ---------- installation ----------
refresh_toolchain_path() {
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
  export PATH="$HOME/.cargo/bin:$PATH"
}

install_node_mac() {
  echo "  → 安装 Node.js"
  if ! command -v brew >/dev/null 2>&1; then
    echo "  \$ NONINTERACTIVE=1 /bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
    if NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"; then
      if [ -x /opt/homebrew/bin/brew ]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
      elif [ -x /usr/local/bin/brew ]; then
        eval "$(/usr/local/bin/brew shellenv)"
      fi
    fi
  fi
  if command -v brew >/dev/null 2>&1; then
    run_cmd brew install node || return 1
  else
    echo "  ! Homebrew 未就绪，无法自动安装 Node.js。请从 https://nodejs.org 安装 LTS 后重跑本脚本。"
    return 1
  fi
}

install_rust() {
  echo "  → 安装 / 升级 Rust"
  if command -v rustup >/dev/null 2>&1; then
    run_cmd rustup update stable || return 1
    refresh_toolchain_path
  else
    echo "  \$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal"
    if curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal; then
      refresh_toolchain_path
    else
      echo "  ! rustup 安装失败，请检查网络或访问 https://rustup.rs 手动安装。"
      return 1
    fi
  fi
}

install_missing() {
  echo
  echo "── 安装缺失的系统依赖 ──"
  local os
  os="$(uname -s)"

  if [ "$os" = "Darwin" ] && ! xcode-select -p >/dev/null 2>&1; then
    echo "  → 触发 Xcode Command Line Tools 安装（系统弹窗，需人工点「安装」）"
    if ! xcode-select --install; then
      echo "  ! xcode-select --install 返回非零，可能是弹窗被取消或已安装，稍后复检确认"
    fi
    CLT_PENDING=1
  fi

  if ! node_ok; then
    if [ "$os" = "Darwin" ]; then
      install_node_mac || echo "  ! Node.js 自动安装未成功，将在复检中报告"
    else
      echo "  ! Linux 请自行用发行版包管理器安装 Node.js >= 18（脚本不改动 Linux 系统环境）。"
    fi
  fi

  if ! rust_ok; then
    install_rust || echo "  ! Rust 自动安装/升级未成功，将在复检中报告"
  fi

  if [ "$CLT_PENDING" -eq 1 ]; then
    if ! xcode-select -p >/dev/null 2>&1; then
      echo
      echo "  ! Xcode CLT 安装需要你在弹窗中完成，完成后请重新运行: bash scripts/setup.sh"
      exit 2
    fi
    echo "  Xcode CLT 已就绪，继续。"
  fi
}

# `generate_context!()` reads src-tauri/icons/icon.png at compile time, even when
# bundle.icon is []. Generate it here rather than relying on `npm run dev` (Tauri's
# beforeDevCommand): there is no race with cargo startup, --no-run leaves a compilable
# tree, and the re-check table below reports the real state.
ensure_dev_icon() {
  local script="$ROOT/scripts/gen-icon.cjs"
  if [ ! -f "$script" ]; then
    echo "  ! 缺少 scripts/gen-icon.cjs，无法生成编译期默认图标"
    return 1
  fi
  echo "  → 检查 / 生成开发态窗口图标 (src-tauri/icons/icon.png)"
  if ! node "$script"; then
    echo "  ! 图标生成失败，generate_context!() 将无法编译"
    return 1
  fi
  return 0
}

ensure_build_icons() {
  local cfg="$ROOT/src-tauri/tauri.conf.json"
  if [ ! -f "$ROOT/src-tauri/icons/icon.icns" ] || [ ! -f "$ROOT/src-tauri/icons/icon.ico" ]; then
    echo
    echo "  ! 打包需要应用图标：准备一张 1024x1024 PNG 后执行"
    echo "      npm run tauri icon path/to/icon.png"
    echo "    再重新运行 bash scripts/setup.sh --build（脚本会自动补全 bundle.icon）"
    echo "    （只想启动开发版可直接 bash scripts/setup.sh，无需图标）"
    exit 2
  fi

  # tauri.conf.json defaults to `"icon": []` so a fresh clone can run `tauri dev`.
  # Before building, fill in the generated icon paths automatically (one-shot build).
  if grep -q '"icon"[[:space:]]*:[[:space:]]*\[\]' "$cfg"; then
    echo "  → 自动补全 tauri.conf.json 的 bundle.icon"
    if ! node --input-type=commonjs -e '
      const fs = require("fs");
      const p = process.argv[1];
      const cfg = JSON.parse(fs.readFileSync(p, "utf8"));
      cfg.bundle.icon = [
        "icons/32x32.png",
        "icons/128x128.png",
        "icons/128x128@2x.png",
        "icons/icon.icns",
        "icons/icon.ico"
      ];
      fs.writeFileSync(p, JSON.stringify(cfg, null, 2) + "\n");
    ' "$cfg"; then
      echo "  ! 自动补全失败，请按 README 第 5 节手动填写 bundle.icon"
      exit 2
    fi
  fi
}

# ---------- main ----------
echo "LogLens 环境自检 · $(date '+%Y-%m-%d %H:%M:%S')"
echo "项目目录: $ROOT"
echo
detect
echo "── 初次检测 ──"
print_table
write_report

if [ "$CHECK_ONLY" -eq 1 ]; then
  echo
  if [ "$HAS_FAIL" -eq 1 ]; then
    echo "存在缺失项，退出码 2。可运行不带参数的脚本自动安装。"
    exit 2
  fi
  echo "环境齐备（--check-only，未安装、未运行）。"
  exit 0
fi

if [ "$NO_SYSTEM_INSTALL" -eq 0 ]; then
  install_missing
else
  echo
  echo "── 跳过系统级安装（--no-system-install）──"
fi

# Make sure the compile-time window icon exists before the re-check, so the table
# reflects reality and `cargo` can compile regardless of which entry point is used.
if command -v node >/dev/null 2>&1; then
  echo
  echo "── 准备编译期资源 ──"
  ensure_dev_icon || { echo "无法生成默认窗口图标，终止。"; exit 2; }
fi

echo
echo "── 复检 ──"
detect
print_table
write_report

if [ "$HAS_FAIL" -eq 1 ]; then
  echo
  echo "仍有缺失项，无法继续。请按上表处理，或查看 README 第 3 节。"
  exit 2
fi

echo
echo "── 安装项目依赖 ──"
run_cmd npm install || { echo "npm install 失败"; exit 2; }

if [ "$MODE" = "build" ]; then
  ensure_build_icons
fi

if command -v cargo >/dev/null 2>&1; then
  echo "  (预热 Rust 依赖，失败不影响后续启动)"
  if (cd "$ROOT/src-tauri" && cargo fetch); then
    echo "  cargo fetch 完成"
  else
    echo "  ! cargo fetch 失败，启动时会重试；若持续失败请检查网络 / 代理"
  fi
fi

if [ "$NO_RUN" -eq 1 ]; then
  echo
  echo "依赖已就绪（--no-run，未启动）。"
  exit 0
fi

echo
case "$MODE" in
  dev)
    echo "── 启动客户端（Ctrl+C 退出）──"
    exec npm run tauri dev
    ;;
  build)
    echo "── 构建发布包 ──"
    exec npm run tauri build
    ;;
  test)
    echo "── 运行验证 ──"
    npm run build || exit 2
    (cd "$ROOT/src-tauri" && cargo test --lib) || exit 2
    echo "全部验证通过。"
    ;;
esac
