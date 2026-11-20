#!/bin/bash
# LogLens 安装脚本（macOS）—— 双击运行，放进 dmg 卷内
#
# 为什么是 .command 而不是 .pkg /.mpkg 安装包：
#   macOS 15 起 PackageKit 要求安装包必须签名，且不支持「只用管理员密码」安装；
#   第三方工具（如 AssinadorPKG）可以给 .pkg 签「自定义证书」，但生成的包只在
#   同样装了那张根证书的机器上才验得过 —— 等于把钥匙串折腾了一遍还是回到同一件事，
#   还多出一次让用户输管理员密码的「安装器同意」弹窗。
#   所以这里仍然是一个脚本：脚本只做两件普通用户本来就能做的事（往 /Applications 拷、
#   清 quarantine），不需要 sudo —— 直接拷不了时才申请一次管理员密码。
#
# 没有 quarantine + 签名完整时，Finder 双击脚本是直接执行的：
#   · 不弹「无法验证开发者」（真正会弹的是 quarantine 里未签名的脚本，
#     而且要等 Gatekeeper 评估十秒才失败）；
#   · 本脚本的签名证书由 dmg 内自带的根证书背书，打包时一并检查；
#   · 如果它仍然被判隔离（IM / 浏览器转发导致），本脚本会继续执行，安装不受影响。
#
# 原来 ad-hoc 的「首次打开-去隔离.command」已废弃：ad-hoc 签名让 Gatekeeper
# 对每次重新签名的 app 都当新二进制，TCC 授权反复弹；改为普通自签证书后有稳定身份。

set +e
# 脚本放在含中文名的可执行文件上时，某些 macOS 版本给的是 -rwx--x--x 这类权限；
# 先确保可读，否则 /bin/bash 读不到脚本会直接报错退出。
chmod u+r "$0" 2>/dev/null || true

VOL="$(cd "$(dirname "$0")" 2>/dev/null && pwd)"
APP_NAME="LogLens"
APP="/Applications/$APP_NAME.app"
ROOT_CN="LogLens Local Development Root"

echo "==================================================="
echo " $APP_NAME 安装"
echo "==================================================="
echo

# ---------- 0. 找到 app 包 ----------
SRC=""
for candidate in "$VOL/$APP_NAME.app" "$VOL/../$APP_NAME.app"; do
  if [ -d "$candidate" ]; then SRC="$candidate"; break; fi
done
if [ -z "$SRC" ]; then
  for candidate in "$VOL"/*.app; do
    if [ -d "$candidate" ]; then SRC="$candidate"; APP="/Applications/$(basename "$candidate")"; break; fi
  done
fi
if [ -z "$SRC" ]; then
  echo "✗ 没找到 $APP_NAME.app。"
  echo "  请把本脚本和 $APP_NAME.app 放在同一个文件夹（dmg 卷）里再运行。"
  echo
  read -r -p "按回车键退出..." _ || true
  exit 1
fi
echo "来源: $SRC"
echo

# ---------- 1. 导入根证书（脚本与 app 签名链的信任锚） ----------
# 根证书在卷内时导入一次；用户机器上第一次遇到这把证书的 app 仍可能弹
# 「无法验证开发者」，导入后后续启动就不会了。失败不阻断安装。
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
[ -f "$KEYCHAIN" ] || KEYCHAIN="$HOME/Library/Keychains/login.keychain"
ROOT_CERT="$VOL/LogLens-Setup-Root.crt"
if [ -f "$ROOT_CERT" ] && ! security find-certificate -c "$ROOT_CN" "$KEYCHAIN" >/dev/null 2>&1; then
  echo "· 导入签名根证书（可选，失败不影响安装）"
  if security add-trusted-cert -d -r trustRoot -k "$KEYCHAIN" "$ROOT_CERT" 2>/dev/null; then
    echo "  完成。卸载: security delete-certificate -c \"$ROOT_CN\""
  else
    echo "  跳过（钥匙串被锁定或已拒绝）"
  fi
fi

# ---------- 2. 拷贝到 /Applications ----------
copy_app() {
  rm -rf "$APP" 2>/dev/null
  /bin/cp -pR "$SRC" "$APP" 2>/dev/null && return 0
  # 系统层面禁止了拷贝目标（SIP / TCC）或权限不足：一次性申请管理员密码
  /usr/bin/osascript <<OSA >/dev/null 2>&1
do shell script "/bin/rm -rf " & quoted form of "$APP" & " && /bin/cp -pR " & quoted form of "$SRC" & " " & quoted form of "$APP" with administrator privileges
OSA
}
echo "· 安装到 /Applications..."
if copy_app; then
  echo "      完成"
else
  echo "      ✗ 拷贝失败（可直接把 $APP_NAME.app 拖进「应用程序」文件夹后双击打开）"
  APP="$SRC"
fi

# ---------- 3. 清隔离并启动 ----------
echo "· 启动 $APP_NAME..."
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true
open "$APP" 2>/dev/null
echo
if [ -d "/Applications/$APP_NAME.app" ]; then
  echo "完成。以后在「启动台 / 应用程序」里直接打开 $APP_NAME 即可。"
  echo "若第一次打开仍提示「无法验证开发者」：右键 $APP_NAME.app → 打开，只需一次。"
fi
sleep 2
exit 0