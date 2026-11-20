#!/usr/bin/env bash
# 生成「本地开发/自分发」用的代码签名证书（macOS）
#
#   scripts/certs/LogLens-Dev        普通签名证书。
#                                    用它签 app，所有构建共享同一个 designated requirement，
#                                    重装后 TCC 授权（本地网络、通知…）不必重新点，
#                                    签名状态也不再是 ad-hoc 的「最低可信」。
#   scripts/certs/LogLens-Installer  安装脚本 setup.command 用。
#                                    关键是 Finder 双击这个脚本时，它不在 quarantine 里、
#                                    而且签名完整，系统可以立刻决断 —— 不需要「右键 → 打开」，
#                                    也不会被挂起十几秒去问 Apple。
#
# 为什么不用 ad-hoc：
#   ad-hoc（codesign -s -）是自签证书里最弱的一种：系统无法证明二进制/脚本没被改过，
#   也不认任何发布者，TCC 授权每次重签（CDHash 变化）都要重新确认。
#   自带根证书的自签证书（根证书装进接收方钥匙串）能建立完整信任链，签名质量与
#   照抄的 Apple 免费开发者证书同档 —— 区别只差 Apple 根本身。
#
# 局限（任何自签证书都一样）：根证书不在 Apple 信任库里，新机器首次遇到这把证书的
#   软件仍可能弹「无法验证开发者」。要彻底免提示只有 Developer ID + 公证；
#   对 Mac 用户来说「右键 → 打开」只需要一次，比手机装描述文件省事得多。
#
# 用法:
#   bash scripts/make-signers.sh              # 缺哪套建哪套；根证书导入登录钥匙串
#   bash scripts/make-signers.sh --force      # 全部重建（会重新生成根证书）
#   bash scripts/make-signers.sh --publish    # 另外导出接收方要装的根证书
#                                             #   scripts/certs/dist/LogLens-Setup-Root.crt
#   bash scripts/make-signers.sh --dest-dir <DIR>   # 产物写到这里（CI 用）
#
# 产物（默认 scripts/certs/，已 gitignore，不进版本库）:
#   LogLens-Root.{key,crt}               根证书 + 根私钥（私钥丢了，已发出的安装脚本就再也验不过）
#   LogLens-Dev.{crt,key,p12}            普通签名证书
#   LogLens-Installer.{crt,key,p12}      安装脚本签名证书
#   dist/LogLens-Setup-Root.crt/.p12     仅 --publish：接收方根证书（不含私钥）
#   SIGNERS-OUTPUT.txt                   证书 SHA-1 指纹
#
# 私钥只留在本机磁盘（600），不交给任何服务端。每个人都可以自己跑一遍生成本机证书；
# 只要接收方机器上也装了同一张根证书，签名链就是闭合的。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CERT_DIR="$ROOT/scripts/certs"
FORCE=0
PUBLISH=0
SKIP_KEYCHAIN="${SKIP_KEYCHAIN:-0}"

while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1 ;;
    --publish) PUBLISH=1 ;;
    --skip-keychain) SKIP_KEYCHAIN=1 ;;
    --dest-dir)
      shift
      [ $# -gt 0 ] || { echo "缺少 --dest-dir 的参数" >&2; exit 3; }
      CERT_DIR="$1"
      ;;
    -h|--help) sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "未知参数: $1" >&2; exit 3 ;;
  esac
  shift
done

[ "$(uname -s)" = "Darwin" ] || { echo "本脚本只能在 macOS 上跑（需要 security / codesign）" >&2; exit 2; }

KEYCHAIN="${HOME}/Library/Keychains/login.keychain-db"
[ -f "$KEYCHAIN" ] || KEYCHAIN="${HOME}/Library/Keychains/login.keychain"

step() { echo; echo "── $* ──"; }
info() { echo "  $*"; }
ok()   { echo "  ✓ $*"; }
warn() { echo "  ! $*"; }
die()  { echo "  ✗ $*" >&2; exit 2; }

# 写一份 openssl 配置：macOS LibreSSL 的 -extfile 不支持以命令行方式追加 section 以外的内容，
# 所以每个证书都单独生成 -extfile，内容直接是扩展段本身。
ext_file() {
  local path="$1"; shift
  printf '%s\n' "$@" > "$path"
}

step "生成签名证书"
mkdir -p "$CERT_DIR"

# ---------- 1. 根证书 ----------
if [ "$FORCE" -eq 1 ] || [ ! -f "$CERT_DIR/LogLens-Root.key" ] || [ ! -f "$CERT_DIR/LogLens-Root.crt" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 3650 \
    -keyout "$CERT_DIR/LogLens-Root.key" -out "$CERT_DIR/LogLens-Root.crt" \
    -subj "/C=US/O=LogLens Local Development/OU=LogLens-Setup-Root/CN=LogLens Local Development Root" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -addext "subjectKeyIdentifier=hash" >/dev/null 2>&1 ||
    die "生成根证书失败"
  rm -f "$CERT_DIR/LogLens-Root.srl"
  chmod 600 "$CERT_DIR/LogLens-Root.key"
  ok "根证书 LogLens-Root.crt（有效期 10 年）"
else
  info "复用已有根证书 $CERT_DIR/LogLens-Root.crt"
fi

# ---------- 2. 叶证书 ----------
# 参数：文件前缀  CN
make_leaf() {
  local stem="$CERT_DIR/$1" cn="$2"
  rm -f "$stem.csr" "$stem.crt" "$stem.p12" "$stem.key" "$stem-ext.cnf"

  openssl req -new -newkey rsa:2048 -nodes -sha256 \
    -keyout "$stem.key.tmp" -out "$stem.csr" \
    -subj "/C=US/O=LogLens Local Development/OU=$1/CN=$cn" >/dev/null 2>&1 ||
    die "生成 $1 的 CSR 失败"

  ext_file "$stem-ext.cnf" \
    "basicConstraints=critical,CA:FALSE" \
    "keyUsage=critical,digitalSignature" \
    "extendedKeyUsage=critical,codeSigning" \
    "subjectKeyIdentifier=hash" \
    "authorityKeyIdentifier=keyid"

  openssl x509 -req -in "$stem.csr" -CA "$CERT_DIR/LogLens-Root.crt" -CAkey "$CERT_DIR/LogLens-Root.key" \
    -CAcreateserial -days 3650 -sha256 -extfile "$stem-ext.cnf" \
    -out "$stem.crt" >/dev/null 2>&1 || die "签发 $1 失败"

  # -legacy：macOS 自带的 LibreSSL 导出的是新版 AES 封装的 p12，security import 解不开
  # （报 MAC verification failed），必须退回 3DES/SHA1 的旧格式。
  openssl pkcs12 -export -legacy -in "$stem.crt" -inkey "$stem.key.tmp" -certfile "$CERT_DIR/LogLens-Root.crt" \
    -name "$cn" -out "$stem.p12" -passout pass:loglens >/dev/null 2>&1 ||
    die "打包 $1.p12 失败"

  mv -f "$stem.key.tmp" "$stem.key"
  rm -f "$stem.csr" "$stem-ext.cnf"
  chmod 600 "$stem.key" "$stem.p12" 2>/dev/null || true
  ok "${cn}（有效期 10 年）"
}

if [ "$FORCE" -eq 1 ] || [ ! -f "$CERT_DIR/LogLens-Dev.p12" ]; then
  make_leaf "LogLens-Dev" "LogLens Development"
else
  info "复用已有 LogLens Dev 证书"
fi
if [ "$FORCE" -eq 1 ] || [ ! -f "$CERT_DIR/LogLens-Installer.p12" ]; then
  make_leaf "LogLens-Installer" "LogLens Installer"
else
  info "复用已有 LogLens Installer 证书"
fi

# ---------- 3. 导入登录钥匙串 ----------
# 根证书先导入：叶证书的签发者必须在钥匙串里，codesign 才能建信任链
if [ "$SKIP_KEYCHAIN" != "1" ] && [ "$(uname -s)" = "Darwin" ]; then
  if ! security find-certificate -c "LogLens Local Development Root" "$KEYCHAIN" >/dev/null 2>&1; then
    security add-trusted-cert -d -r trustRoot -k "$KEYCHAIN" "$CERT_DIR/LogLens-Root.crt" ||
      die "导入根证书失败（钥匙串被锁定？）"
    ok "根证书已导入并设为信任"
  else
    info "根证书已在钥匙串中"
  fi

  import_p12() {
    local stem="$1" name="$2" out
    if ! out="$(security import "$stem.p12" -k "$KEYCHAIN" -P loglens -T /usr/bin/codesign -T /usr/bin/security 2>&1)"; then
      case "$out" in
        *"already exists"*|*"已存在"*) info "$name 已在钥匙串中"; return 0 ;;
        *) die "导入 $name 失败: $out" ;;
      esac
    fi
    ok "$name 已导入登录钥匙串"
  }
  import_p12 "$CERT_DIR/LogLens-Dev" "LogLens Development"
  import_p12 "$CERT_DIR/LogLens-Installer" "LogLens Installer"

  # 关键一步：没有 partition list 授权，codesign 读私钥时会弹「是否允许」，
  # 批处理里就表现为卡住或 find-identity 里看不到这张证书。
  # 这一步必须解锁登录钥匙串（就是开机密码）；自动化环境传 KEYCHAIN_PASSWORD，
  # 否则钥匙串已解锁时用 -k "" 试一次，失败就明确提示用户自己敲一次。
  if [ -n "${KEYCHAIN_PASSWORD:-}" ]; then
    security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null 2>&1 || true
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
      -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null 2>&1 &&
      ok "私钥分区已授权（codesign 不再弹窗）" ||
      warn "私钥分区授权失败，codesign 首次使用私钥时可能弹窗，点「始终允许」即可"
  elif security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "" "$KEYCHAIN" >/dev/null 2>&1; then
    ok "私钥分区已授权（codesign 不再弹窗）"
  else
    warn "无法自动授权私钥分区（需要登录钥匙串密码）。"
    info "  请手动执行一次（按提示输入密码）:"
    info "  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k <密码> $KEYCHAIN"
    info "  或设置环境变量 KEYCHAIN_PASSWORD 后重跑本脚本。"
  fi
fi

# ---------- 4. 指纹记录 ----------
{
  echo "LogLens-Dev: $(openssl x509 -in "$CERT_DIR/LogLens-Dev.crt" -noout -fingerprint -sha1 | sed 's/^.*=//')"
  echo "LogLens-Installer: $(openssl x509 -in "$CERT_DIR/LogLens-Installer.crt" -noout -fingerprint -sha1 | sed 's/^.*=//')"
  echo "Root: $(openssl x509 -in "$CERT_DIR/LogLens-Root.crt" -noout -fingerprint -sha1 | sed 's/^.*=//')"
} > "$CERT_DIR/SIGNERS-OUTPUT.txt"
ok "指纹: $CERT_DIR/SIGNERS-OUTPUT.txt"

# ---------- 5. 接收方根证书 ----------
if [ "$PUBLISH" -eq 1 ]; then
  mkdir -p "$CERT_DIR/dist"
  cp -f "$CERT_DIR/LogLens-Root.crt" "$CERT_DIR/dist/LogLens-Setup-Root.crt"
  openssl pkcs12 -export -legacy -in "$CERT_DIR/LogLens-Root.crt" -nokeys \
    -name "LogLens Setup Root" -out "$CERT_DIR/dist/LogLens-Setup-Root.p12" -passout pass:loglens >/dev/null 2>&1 ||
    die "导出根证书失败"
  chmod 644 "$CERT_DIR/dist/LogLens-Setup-Root.crt" "$CERT_DIR/dist/LogLens-Setup-Root.p12"
  root_fp="$(openssl x509 -in "$CERT_DIR/dist/LogLens-Setup-Root.crt" -noout -fingerprint -sha256 |
    sed 's/^.*=//; s/://g' | tr 'A-F' 'a-f')"
  echo "$root_fp" > "$CERT_DIR/LOG_LAYOUT_DEV_ROOT_SHA256"
  cat > "$CERT_DIR/dist/README.txt" <<EOF
LogLens 安装脚本的签名根证书（仅证书，无私钥）

SHA-256: $root_fp

接收方机器上要装的就是这一张。双击 dmg 卷内的「安装 LogLens.command」会自动导入；
也可以手工执行：
  security add-trusted-cert -d -r trustRoot -k ~/Library/Keychains/login.keychain-db LogLens-Setup-Root.crt

卸载：
  security delete-certificate -c "LogLens Local Development Root"
EOF
  ok "接收方根证书: $CERT_DIR/dist/LogLens-Setup-Root.crt （SHA-256 ${root_fp}）"
fi

# ---------- 6. 自检 ----------
echo
if command -v codesign >/dev/null 2>&1 && security find-identity -v -p codesigning 2>/dev/null | grep -q 'LogLens Development'; then
  ok "codesign 已识别 'LogLens Development'"
elif [ "$SKIP_KEYCHAIN" = "1" ]; then
  info "已跳过钥匙串导入（SKIP_KEYCHAIN=1）"
else
  warn "codesign 暂未识别 'LogLens Development'：请解锁登录钥匙串后重试"
fi
echo
info "下一步: bash scripts/package.sh --bundles app,dmg"
info "打包脚本会自动发现 'LogLens Development' / 'LogLens Installer' 并用它们签名；缺失时回退 ad-hoc 并告警。"