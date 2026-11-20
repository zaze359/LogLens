# LogLens · 日志分析客户端

分析各种采集日志的桌面客户端。首个版本聚焦 **Android Logcat 本地文件**：文件解析放在 Rust 侧，Vue3 负责交互，规则与模板可以快速把海量日志收敛到几条关键线索。

> 目标平台：macOS、Windows（同一套代码，分别在对应系统打包）。技术栈：Tauri 2 + Rust + Vue 3 + TypeScript + Vite。

---

## 1. 本期范围

| 能力 | 状态 |
| --- | --- |
| 日志来源 | ✅ 本地文件（.log / .txt / .logcat）+ 直连 Android 设备（adb 实时抓取） |
| 设备工作区 | ✅ adb 探测、设备列表（授权 / 未授权 / 离线）、**设备作为工作区标签**：左侧竖排工具栏（日志 / 抓包 / 后续 adb 工具），切换即点图标且采集不中断；关掉设备会连同它的日志一起关闭 |
| 设备网络请求 | ✅ WebView CDP（H5 明文）+ 内建代理（原生明文 HTTP、HTTPS 隧道元数据）+ HAR / JSONL 导入、复制 cURL（凭据按原文保存，脱敏可开关） |
| macOS / Windows 桌面客户端 | ✅ Tauri 2，双平台构建脚本一致 |
| Android Logcat 分析 | ✅ 流式解析、多格式兼容、包名从 pid 反推 |
| 快捷筛选：包名 / 级别 / Tag / PID / 关键字 / 正则 / 排除 / 时间范围 / 排序 | ✅ |
| 搜索与筛选分离 | ✅ 同一个关键字可切换「筛选」（只显示匹配）或「查找」（只高亮定位，不改结果集） |
| 自定义规则：多条件 AND·OR、字段 & 操作符、取反、正则、大小写 | ✅ 可视化编辑器 + 命中数测试 |
| 内置快捷模板 | ✅ 10 个，归入 4 个分组（通用速查 / 稳定性排查 / 功能链路 / Launcher） |
| 规则分组（分类） | ✅ 一等公民分组，纯分类不绑定应用：可建 / 改 / 删 / 折叠；内置 4 个（通用速查 / 稳定性排查 / 功能链路 / Launcher） |
| 规则搜索 | ✅ 侧栏实时多关键词搜索，覆盖规则名 / 说明 / 分组名 / 条件字段与匹配值 |
| 规则持久化与导入导出 | ✅ JSON v2（分组 + 规则一起），兼容旧的 v1 与纯规则数组 |
| 结果导出 | ✅ 命中日志原样导出为 .log |
| 多标签 | ✅ 同时开 4 个日志（文件 / 设备实时流混搭），各自独立筛选与滚动位置，切走后实时流继续接收 |
| 大文件友好 | ✅ 虚拟滚动 + 后端分页 + 命中索引缓存 |
| 长行完整显示 | ✅ 消息列按最长行自适应宽度，整表左右滚动，不再用省略号截断 |
| 导入进度 | ✅ 后台解析 + 按字节的进度卡片，可取消，导入期间界面不冻结 |
| 客户端诊断日志 | ✅ 故障 + 各模块流程快照统一落盘，顶栏入口查看、筛选、复制、导出 |
| 无线配对 / 多文件合并 / 压缩文件 / 其他日志类型 | ⛔ 规划中（见第 13 节） |

---

## 2. 目录结构

```
.
├── index.html                    # Vite 入口
├── package.json                  # 前端依赖 + tauri 脚本
├── vite.config.ts                # 固定 1420 端口（与 devUrl 对应）
├── src/
│   ├── main.ts / App.vue         # 应用外壳、拖放事件
│   ├── types.ts                  # 与 Rust 模型一一对应的 IPC 类型
│   ├── lib/ipc.ts                # invoke 封装（camelCase → snake_case 参数）
│   ├── stores/session.ts         # 多标签状态机、查询/分页/规则/导入导出动作
│   ├── components/               # 顶栏、标签条、规则侧栏、分组/条件编辑器、筛选栏、统计、虚拟表格、详情抽屉
│   │   └── device/               # 设备工作区：DevicePickerDialog（选设备）+ DevicePage（工作区）+ 各功能 Tab
│   └── styles/main.css           # 深色主题
├── src-tauri/
│   ├── Cargo.toml / build.rs / tauri.conf.json / capabilities/default.json
│   ├── templates.json            # 内置分组 + 快捷模板（纯 JSON，编译期嵌入二进制）
│   └── src/
│       ├── model.rs              # 前后端共享的数据结构（camelCase）
│       ├── logcat.rs             # Logcat 解析器（含流式增量解析）+ 单元测试
│       ├── rules.rs              # 规则编译与匹配 + 单元测试
│       ├── builtin.rs            # 加载模板 + 模板合法性单测
│       ├── adb.rs                # adb 定位、设备枚举、logcat 参数构造、shell / forward 辅助
│       ├── stream.rs             # 实时 adb logcat 子进程与读线程
│       ├── network.rs            # 网络请求模型、HAR / JSONL 解析、敏感字段脱敏
│       ├── cdp.rs                # 零依赖 HTTP/1.1 + WebSocket CDP 客户端、Network.* 事件组装
│       ├── netprobe.rs           # 设备抓包能力体检与四条路线评估
│       ├── netcapture.rs         # CDP 实时采集线程、代理 ProxyGuard（停止即清空）、统一采集抽象 AnyCapture
│       ├── proxy.rs              # 内建 HTTP/CONNECT 代理：明文 HTTP 记录、HTTPS 隧道 + SNI
│       ├── session.rs            # 日志会话、网络请求会话、分页查询、命中索引缓存
│       ├── config_store.rs       # 自定义规则持久化
│       ├── commands.rs           # Tauri IPC 命令
│       └── lib.rs / main.rs      # 应用入口
├── scripts/
│   ├── setup.sh / setup.ps1      # 一键检测 / 安装 / 运行
│   ├── package.sh / package.ps1  # 一键打包，产物收集到 release/<平台>/
│   ├── gen-icon.cjs              # 生成 Tauri 编译期默认窗口图标（零依赖、幂等）
│   └── mitmproxy_jsonl.py        # mitmproxy add-on：导出可导入的 JSONL 抓包
├── samples/logcat-sample.txt     # 示例日志，覆盖多格式 / 堆栈 / ANR / 崩溃
└── .trellis/tasks/11-12-loglens-mvp/   # 需求 / 设计 / 实现记录
```

---

## 3. 环境要求

| 依赖 | 版本 | 说明 |
| --- | --- | --- |
| Node.js | ≥ 18（建议 20 LTS） | `node -v` |
| Rust | **≥ 1.90** | `rustc --version`；Tauri 2 官方 dialog 插件要求 1.90+ |
| macOS | Xcode Command Line Tools | `xcode-select --install` |
| Windows | MSVC Build Tools + WebView2 Runtime | 安装 VS Build Tools（C++ 工作负载）；Win11 自带 WebView2 |

检查命令：

```bash
node -v && npm -v
rustc --version && cargo --version
```

---

## 4. 快速开始（推荐：一键脚本）

| 系统 | 命令 |
| --- | --- |
| macOS | `bash scripts/setup.sh` |
| Windows | `powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1` |

一条命令完成 **环境检测 → 缺失依赖自动安装 → 项目依赖安装 → 启动客户端**。已有 Node 时也可用 `npm run setup` / `npm run setup:win` / `npm run check`。

**检测项**：Node ≥ 18、npm、Rust ≥ 1.90、cargo、rustup；macOS 额外检查 Xcode Command Line Tools / Homebrew；Windows 额外检查 MSVC C++ 生成工具、WebView2 Runtime、winget；两端都检查 1420 端口占用、应用图标、`node_modules`。结果以表格输出，`FAIL` 会阻塞启动。

**自动安装**：macOS 缺 Node 用 Homebrew（必要时非交互安装 Homebrew），Rust 缺失或版本过低用 rustup，缺 CLT 触发系统安装弹窗并提示完成后重跑；Windows 用 winget 装 Node LTS / VS BuildTools / WebView2，用 rustup-init 装 Rust。之后统一执行 `npm install` 与 `cargo fetch` 预热。脚本幂等，可反复执行。Linux 仅做检测与项目依赖安装，不自动改动系统包管理器。

| 功能 | macOS / Linux | Windows |
| --- | --- | --- |
| 只检测不安装不运行（CI 可用退出码 0/2） | `--check-only` | `-CheckOnly` |
| 跳过系统级安装（仍装项目依赖） | `--no-system-install` | `-NoSystemInstall` |
| 装完不启动 | `--no-run` | `-NoRun` |
| 启动开发版（默认行为） | `--dev` | 默认即 dev，无需参数 |
| 打包（转调 `scripts/package.sh`，产物在 `release/`） | `--build` | `-Build` |
| 跑类型检查 + Rust 单测 | `--test` | `-Test` |
| 输出 Markdown 报告 | `--report <path>` | `-Report <path>` |
| 帮助 | `--help` | `-Help` |

```bash
bash scripts/setup.sh --check-only --report ./setup-report.md   # 只看体检报告
bash scripts/setup.sh --no-system-install --no-run               # 不改动系统环境
bash scripts/setup.sh --test                                    # 检测 + 安装 + 验证
```

> 系统级安装会改动本机环境（Homebrew / rustup / winget 包）。不想让脚本动系统，加 `--no-system-install` / `-NoSystemInstall`，或先跑 `--check-only` / `-CheckOnly` 看报告再决定。

### 手动步骤（不使用脚本）

```bash
cd /path/to/LogAnalyse
npm install
npm run tauri dev        # 启动桌面客户端（会先拉起 Vite dev server）
```

浏览器预览：`npm run dev` 只渲染界面，本地文件读写与规则持久化不可用（界面会提示），请用 `npm run tauri dev`。

首次体验：打开 `samples/logcat-sample.txt`，然后依次点击左侧模板，观察命中数与级别分布变化。

---

## 5. 打包（macOS / Windows / Linux）

**开发态图标（自动生成，无需操作）**：Tauri 2 的 `generate_context!()` 在编译期会读取 `src-tauri/icons/icon.png` 作为默认窗口图标，**即使 `bundle.icon` 是 `[]` 也一样**。因此 `npm install`（`postinstall`）、`npm run dev`、`npm run build`、`npm run tauri dev/build` 以及一键脚本都会执行 `scripts/gen-icon.cjs`，用纯 Node 标准库生成一张 64×64 占位渐变 PNG（幂等，不会覆盖你已有的图标）。新克隆的仓库可以直接启动，不需要提交二进制图标；`npm install` 之后直接执行 `cd src-tauri && cargo test --lib` 也能编译通过。

> 只有**完全没有跑过 npm 脚本**、直接在干净仓库里调用 `cargo build` 才会因为缺 `icon.png` 失败，此时补跑一次 `node scripts/gen-icon.cjs` 即可。

**正式应用图标**：`src-tauri/icons/` 下的 `icon.png` / `32x32.png` / `128x128.png` / `128x128@2x.png` / `icon.icns` / `icon.ico` 已换成 LogLens 正式 logo（蓝色圆角方 + Android 机器人从日志面板探头 + 红黄绿三条日志行）。矢量源文件与生成脚本在 `design/logo/`（`logo-final.svg` + `render_icon.py`，纯 Pillow、无额外依赖），详见 `design/logo/README.md`。`gen-icon.cjs` 不会覆盖这些已存在的图标；打包脚本会自动注入 `bundle.icon`，无需改 `tauri.conf.json`。

**一步打包（推荐）**：`scripts/package.sh` / `scripts/package.ps1` 把「编译 → 收集产物 → 写构建信息 → 生成校验和」串成一条命令，产物统一落到项目根目录的 `release/<平台>/`，不用再去 `src-tauri/target/release/bundle/` 里翻：

| 系统 | 命令 | 输出目录 |
| --- | --- | --- |
| macOS | `bash scripts/package.sh`（或 `npm run package`） | `release/macos/`：`LogLens.app`、`LogLens_0.1.0_aarch64.dmg`（卷内含 app、Applications 链接、`安装 LogLens.command`、`使用说明.txt`，用自签证书并 `--publish` 导出根证书时还会带上 `LogLens-Setup-Root.crt`）、`LogLens_0.1.0_aarch64.zip` |
| Windows | `powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1`（或 `npm run package:win`） | `release/windows/`：`LogLens_0.1.0_x64_en-US.msi`、`LogLens_0.1.0_x64-setup.exe` |

每个平台目录都会附带 `BUILD-INFO.md`（版本 / 平台 / Git 提交 / 工具链 / 文件清单）和 `SHA256SUMS.txt`（校验和）。`release/` 已在 `.gitignore` 中，不会被误提交。

| 选项 | macOS / Linux | Windows | 作用 |
| --- | --- | --- | --- |
| 清空本次平台目录后重打 | `--clean` | `-Clean` | 只删 `release/<平台>/` 里的旧产物；输出目录必须在项目 `release/` 下，否则拒绝执行 |
| 不重新编译，只收集现有产物 | `--skip-build` | `-SkipBuild` | 复用 `src-tauri/target/release/bundle/` |
| 换输出根目录 | `--out <dir>` | `-Out <dir>` | 默认 `release/`；配合 `--clean` 时仍须位于项目 `release/` 下 |
| 指定打包格式 | `--bundles app,dmg` | `-Bundles msi,nsi` | 直接透传给 `tauri build --bundles`，逐个格式执行 |
| 指定 macOS 签名身份 | `--identity <名称或哈希>` | — | 默认自动挑：Apple 开发者证书 → 本机自签「LogLens Development」→ ad-hoc |
| 跳过重签名 | `--no-identity` | — | 只保留 tauri 的 linker 级签名；Gatekeeper 可能判「已损坏」 |
| 不注入自定义图标 | `--no-icons` | `-NoIcons` | 使用 Tauri 通用占位图标 |
| 完成后打开输出目录 | `--open` | `-Open` | Finder / 资源管理器 |
| 帮助 | `--help` | `-Help` | 列出全部选项 |

脚本的几个约定：

- **某一种格式打包失败不阻断交付**。脚本按 `--bundles` 逐个格式执行，全部失败才报错。DMG 依赖 `hdiutil` 挂载磁盘卷，在无权限 / 沙箱 / CI 环境下会失败；NSIS / AppImage 也可能因缺工具链失败，此时已产出的 `.app` / `zip` / `msi` / `deb` 照常交付并明确告警。
- **`app` 格式最后打**。`tauri build --bundles dmg` 结束时会删掉为它临时生成的 `bundle/macos/*.app`，所以脚本把 `app` 挪到执行顺序末尾，保证 `.app` 与便携 zip 都能收集到（其余格式顺序仍按 `--bundles`）。
- **macOS 收尾会签名并重建 dmg**。先按「`--identity` → Apple 开发者证书 → 本机自签证书「LogLens Development」→ ad-hoc」挑一个签名身份，用 `codesign --force --deep` 给 `.app` 完整重签（tauri 默认只给可执行文件做 linker 签名，Gatekeeper 会判「已损坏」）；再用 `hdiutil` 重建 dmg，把 `安装 LogLens.command`（用「LogLens Installer」证书签名）和 `使用说明.txt` 放进卷内；若本机用 `make-signers.sh --publish` 导出过接收方根证书，还会把 `LogLens-Setup-Root.crt` 放进卷内供安装脚本导入。重建时**丢掉 tauri 的 `.DS_Store`**：它只给 `LogLens.app` 和 `Applications` 记了图标坐标和窗口大小，新增的文件没有坐标，Finder 会把它们排在窗口可视区之外（要滚动才看得到，接收方会以为卷里没有）；删掉后 Finder 自动排列，所有条目一眼可见。卷名也带上版本号（`LogLens 0.1.0`），避免 Finder 按卷名继承旧窗口尺寸。任一步失败只告警，不影响已产出的其它文件。
- **只收集当前版本的产物**。`bundle/` 不会自动清历史版本，而 dmg / msi / deb / AppImage 文件名带版本号；脚本只复制文件名含当前 `version` 的产物，跳过旧版本并告警。
- **不写回 `tauri.conf.json`**。图标齐全时通过 `tauri build --config` 一次性注入 `bundle.icon`（Windows 上写入临时 JSON 后传路径），受版本控制的配置保持干净；`setup.sh --build` / `setup.ps1 -Build` 也改为直接转调打包脚本。
- **图标可以偷懒**。把一张 1024×1024 的 `app-icon.png` 丢到项目根目录，脚本会自动执行 `tauri icon` 生成整套图标并注入；没有图标时也能出包，只是使用 Tauri 通用占位图标（并给出告警）。
- **cargo 不在 PATH 时自动兜底**。从 IDE / GUI 启动的终端可能没加载 `~/.cargo/env`，脚本会尝试 `$HOME/.cargo/bin`、`/opt/homebrew/bin`、`/usr/local/bin`（Windows 为 `%USERPROFILE%\.cargo\bin`）；仍找不到时会提示执行 `source "$HOME/.cargo/env"` 后重试。
- **签名状态**。macOS 默认用本机自签证书（`bash scripts/make-signers.sh` 生成）签名，没有证书时回退 ad-hoc；均未做 Apple Developer ID 签名与公证。Windows 未签名（SmartScreen 可能提示「未知发布者」）。BUILD-INFO 会写明本次实际使用的签名身份。macOS 包的接收方指引见下方「分发给他人」。
- **zip 便携包用 `ditto` 打包**，保留资源分叉，解压后双击即可运行。

**打 tag 自动出包（GitHub Actions）**：推一个 `v` 开头的 tag，GitHub 会跑 `.github/workflows/release.yml`，分别在 macOS / Windows / Linux 上构建，并把三个平台的压缩包挂到对应的 GitHub Release。构建步骤直接调用 `scripts/package.sh` / `scripts/package.ps1`，所以 CI 产物与本地打包的目录结构、`BUILD-INFO.md`、`SHA256SUMS.txt` 完全一致。

```bash
# 1) 先同步版本号（tauri.conf.json 决定产物文件名和 tag 校验，务必一起改）
#    src-tauri/tauri.conf.json  version
#    package.json               version
#    src-tauri/Cargo.toml       version（只影响二进制元信息，CI 不校验）
git commit -am "chore(release): 0.1.0"

# 2) 打 tag 并推送（tag 必须是 v + tauri.conf.json 的 version，否则 CI 第一步就失败）
git tag v0.1.0 && git push origin v0.1.0
```

| 平台 | runner | Release 资产 | 包内文件 |
| --- | --- | --- | --- |
| macOS（Apple 芯片） | `macos-latest` | `LogLens_0.1.0_macos_aarch64.zip` | `.app`、`.dmg`、便携 zip、`安装 LogLens.command`、`使用说明.txt`、`BUILD-INFO.md`、`SHA256SUMS.txt` |
| Windows（x64） | `windows-latest` | `LogLens_0.1.0_windows_x64.zip` | `.msi`、`-setup.exe`、`BUILD-INFO.md`、`SHA256SUMS.txt` |
| Linux（x64） | `ubuntu-22.04` | `LogLens_0.1.0_linux_x64.tar.gz` | `.deb`、`.AppImage`、`BUILD-INFO.md`、`SHA256SUMS.txt` |

- 每个平台的产物单独打包成一个资产，`SHA256SUMS.txt` 与包内文件一一对应，不会被其它平台的同名文件覆盖。
- 某一平台失败不会阻断其它平台（`fail-fast: false`）；Release 在所有平台跑完后一次性创建。
- 版本号带 `-` 后缀（如 `0.2.0-beta.1`）会自动标为 **pre-release**。
- 在 Actions 页面手动触发（`workflow_dispatch`）只构建、上传 workflow 产物，**不创建 Release**，用来试跑工作流。
- GitHub 托管的 Intel Mac runner 已下线，macOS 只出 aarch64 包；需要 Intel 包时改用 `--target universal-apple` 之类的通用二进制构建（`package.sh` 目前不透传 `--target`，需手工执行 `npm run tauri build` 并补 `rustup target add x86_64-apple-darwin`）。
- Release 说明里写明的签名状况：CI runner 上没有本机证书，macOS 产物是 ad-hoc 签名 / 未公证；Windows、Linux 未签名。想要稳定签名身份时在本地跑 `bash scripts/make-signers.sh` 后自行打包。

**客户端更新（GitHub Release）**：客户端内置 `tauri-plugin-updater`，顶栏的版本号按钮打开更新面板，读取仓库最新 Release 的 `latest.json`；发现更高版本时在应用内下载、安装并重启。启动后会在后台静默检查一次。

- 更新包用 Tauri 的 minisign 密钥签名（与 Apple 签名无关）。公钥写死在 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，私钥只放 GitHub Secrets，不进仓库。
- 首次使用前在仓库配置 secret（私钥在本机 `~/.tauri/loglens.key`，**务必备份**：丢了就没法再给已安装的用户发更新）：

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/loglens.key
# 私钥带密码时再配置：gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

- 本地想产出更新包：`export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/loglens.key)"` 后再跑打包脚本，更新包落在 `release/updater/`；没有设置时脚本会跳过并在日志里说明。
- Release 里会多出 `LogLens_<版本>_aarch64.app.tar.gz`（macOS）、`LogLens_<版本>_amd64.AppImage`（Linux）、`LogLens_<版本>_x64-setup.exe`（Windows）和 `latest.json`；`.sig` 的正文内嵌进 `latest.json`，不再单独上传。
- 客户端只比较版本号，所以**发版前必须递增 `src-tauri/tauri.conf.json` 的 version**，否则装在旧版本上的用户看不到更新。

**本地签名证书（macOS，可选）**：`scripts/make-signers.sh` 在本机生成一套自签证书，打包脚本会自动挑用：

```bash
bash scripts/make-signers.sh            # 生成根证书 + 「LogLens Development」/「LogLens Installer」签名证书，并导入登录钥匙串
bash scripts/make-signers.sh --publish  # 额外导出接收方要装的根证书 scripts/certs/dist/LogLens-Setup-Root.crt
```

- 证书与私钥都落在 `scripts/certs/`（已 gitignore，绝不入库）；`--force` 全部重建、`--dest-dir <dir>` 改输出目录（CI 用）、`--skip-keychain` 只生成不导入。
- 自签证书让 app 有稳定的签名身份：重装后 TCC 授权（本地网络、通知…）不必重新确认；接收方机器没装根证书时，首次打开仍可能提示一次「无法验证开发者」，右键 → 打开即可。

**分发给他人（macOS 必读）**：产物没有 Apple Developer ID 签名与公证（默认本机自签，没有证书时回退 ad-hoc），打包脚本在收尾阶段专门兜住了这个坑：

- **`.app` 做一次完整重签名**。tauri 默认只给可执行文件做 linker 阶段的 ad-hoc 签名（`codesign` 显示 `Info.plist=not bound`、`spctl` 报 `code has no resources but signature indicates they must be present`），这种签名在 Gatekeeper 眼里是坏的，会直接判「已损坏，无法打开，你应该将它移到废纸篓」，用户连「仍要打开」的入口都没有。重签后错误降级为可覆盖的「无法验证开发者」。
- **`安装 LogLens.command` 与 `使用说明.txt` 放进 dmg 卷内**。只发 dmg 时接收方也能拿到安装脚本：双击即可清隔离、把 app 拷进「应用程序」并启动，同时导入卷内根证书（有的话）。

接收方操作：

1. **装**：双击 dmg，双击卷内的 `安装 LogLens.command`；也可以把 `LogLens.app` 拖进右侧「应用程序」，再从「应用程序」启动（首次需右键 → 打开）。
2. **首次打开被拦下**：
   - 提示「无法验证开发者」→ 右键 `LogLens.app` → 打开；或「系统设置 → 隐私与安全性」底部点「仍要打开」。
   - 提示「已损坏」→ 多半是 IM 传输带上的隔离标记，文件本身没损坏；双击卷内的 `安装 LogLens.command` 即可。
3. **手工替代**（对方愿意用终端时）：`xattr -cr /Applications/LogLens.app` 后重新打开。

> 经微信、QQ、飞书等 IM 传输会给文件打上 `com.apple.quarantine` 隔离属性 —— **文件本身没有损坏**。发之前用 zip 分发包里的 `SHA256SUMS.txt` 核对，一致即说明镜像完好。

> 该包为 **aarch64**，只能运行在 Apple 芯片（M 系列）Mac；Intel Mac 需另行构建 x64 包。彻底免除以上步骤需加入 Apple Developer 计划，对产物做 Developer ID 签名 + 公证（notarization）。

**第一步：生成正式打包图标**（可选但推荐）：

```bash
npm run tauri icon path/to/icon.png     # 建议 1024x1024 PNG，生成 src-tauri/icons/*
```

脚本会自动把 `bundle.icon` 注入到本次构建（不修改配置文件），因此**不需要手动改 `tauri.conf.json`**。只有当你绕过脚本、直接执行 `npm run tauri build` 且希望产物带图标时，才需要把上面生成的图标路径写进 `tauri.conf.json` 的 `bundle.icon`；`icon: []` 是开发态默认值，保证新克隆的仓库能直接 `tauri dev`。

**第二步：在目标平台上手动构建**（不经脚本时的原始命令；Tauri 不做跨系统打包，mac 出 dmg、Windows 出 msi 需要各自系统或 CI）：

```bash
# macOS
npm run tauri build
# 产物：src-tauri/target/release/bundle/dmg/*.dmg、macos/*.app

# Windows（PowerShell / cmd）
npm run tauri build
# 产物：src-tauri/target/release/bundle/msi/*.msi、nsis/*.exe
```

`npm run build` 会先执行 `vue-tsc --noEmit` 做类型检查，再产出 `dist/`。

---

## 6. 两种工作区：Android 设备工作区 / 本地文件工作区

**日志分析界面是共用的，工作区不是。** 规则侧栏、筛选、统计、表格、详情这一套只负责「显示一份日志」，不关心它来自本地文件还是设备上跑的 logcat —— 两者走同一套解析器、规则、筛选与导出。不同的是**工作区**：它们决定「日志从哪来、这台设备还能做什么」。

顶栏右侧是**两个对称的快捷入口**，各自创建一个工作区标签：

| 入口 | 创建什么 | 里面有什么 |
| --- | --- | --- |
| 「打开日志文件」 | **本地文件工作区**（一个日志一个标签） | 就是日志分析界面；多文件 = 标签条里多个工作区，切换文件就是点标签 |
| 「设备」 | **设备工作区**（一台设备一个标签） | 左侧工具栏 + 内容区：日志 / 抓包 / 之后的 adb 工具（安装包、shell、截图…） |

```
顶栏 [清空筛选][导出命中结果][打开日志文件] │ [设备]
──────────────────────────────────────────────────────────────
标签条  [▣ emulator-5554 设备] │ [xxx.log 5.2 万行]
──────────────────────────────────────────────────────────────
         ┌──┐
         │▤ │ 日志（默认）      ┐
         │🌐│ 抓包              ├─ 右侧内容区：日志面板 / 请求表
         └──┘                   ┘
```

### 日志和抓包是设备里的「工具」，不是并列的工作区标签

一台设备的实时日志，本来就是「这台设备抓到的日志」。把它和设备标签并排放在标签条上，标题里又都带着设备名（`emulator-5554` / `emulator-5554 · 实时 logcat`），看起来就是两个说同一件事的重复标签。

所以**标签条上只有工作区**，日志进了设备里面：

- 设备工作区左边一条竖排工具栏（`DeviceToolRail.vue`），工具列表在 `components/device/operations.ts` 里只有一份：`日志` / `抓包`。选中是实心蓝块，右侧看内容。
- 「日志」工具渲染的就是**共用的日志面板**（`LogPane.vue`，全应用只有一份实例，靠 `v-show` 显隐）。本地文件工作区用的是同一个组件 —— 显示逻辑一样，数据来源不同。
- 日志的归属靠 `info.device`（设备序列号）判断：切到一份设备日志 = 进那台设备工作区 + 选中「日志」工具（`followTabOwner()`），不需要额外记状态。
- 关掉设备工作区会**连同它的日志一起关闭**（停 `adb logcat` 子进程、释放后端会话）再把设备代理清空为直连。日志属于设备，就不该出现「设备已经没了、日志标签还挂着」的状态。
- 本地文件没有设备：标签条上的标签点进去直接是日志面板，左侧没有工具栏。

### 为什么是「工作区标签 + 工具条」，而不是弹窗 / 停靠面板 / 一排功能标签

抓包和看日志是同一件事的两面，用户要在两者之间来回看。凡是把其中一个盖住的形态（弹窗）、抢走一块空间的形态（停靠面板）、或者把功能摊成**第二排横向标签**的形态，都会让「它是设备里的东西」这件事变得模糊。

- 设备工作区**一开就待在标签条里**，工作区之间切换就是点标签；设备内部的东西都在左侧工具栏里，一眼看出「这些都属于这台设备」。
- 切走工具时**只是隐藏，不卸载** —— 日志面板和抓包页都还在，采集继续跑。
- 关掉设备工作区标签 = 关掉它的日志 + 停止采集 + 将设备代理清空为直连（`closeDeviceWorkspace()`），不是单纯隐藏视图。
- 一台设备一个工作区：选同一台设备是**聚焦**已有工作区，不会开出重复标签；两台设备就是两个工作区标签，各自有自己的工具选中状态（`store.deviceOps`）。

### 设备选择只在「创建 / 换设备」时发生

「设备」入口打开**选择器**：列 adb 看到的所有设备、刷新、显示授权 / 离线、标记哪些已有工作区。**这是唯一选设备的地方**，且它做的事是「创建一个工作区」，不是「给某个面板换个参数」。

`refreshDevices()` 在启动时静默跑一次（adb 没装不报错），让选择器一打开就是最新的设备列表。它**不会**改动当前显示的工作区——那是用户的决定，刷新不能悄悄把界面内容换掉。

| 工具 | 做什么 | 依赖的 adb 能力 |
| --- | --- | --- |
| 日志 | 实时跟随 `adb logcat`，内容显示在共用日志面板里 | `shell logcat`（长驻子进程） |
| 抓包 | WebView CDP 采集 / 内建代理采集 / 导入 HAR | `forward`、`shell settings`、本机 LAN 代理 |

**怎么加一个设备工具**：

1. 在 `src/components/device/` 下新建 `XxxTab.vue`，`defineProps<{ serial: string }>()`，自己负责自己的控件和「开始」按钮——不要往页面底部塞按钮，否则每加一个都要改外壳。
2. 在 `components/device/operations.ts` 里加一条（`id` / `label` / `hint`），`DeviceOpId` 里加同名值；工具栏按这份列表渲染图标。
3. 在 `DevicePage.vue` 里加一个 `v-show` 区块（**必须 `v-show`，不能用 `v-if`**：切工具会卸载组件，正在跑的采集会被直接中断）。如果这个工具的内容是「日志」这种共用面板，就不放进 `DevicePage`，而是像 `LogPane` 一样在 `App.vue` 里按 `currentOp` 显隐。
4. 如果这项功能会在设备上留下状态（全局代理、端口转发、临时文件），实现 `shutdown()` 并加进 `shutdownOperations()`——关闭工作区、退出窗口都会走它；只切工具不会停掉它。

抓到的日志与本地文件走**同一套解析器、规则、筛选与导出**，只是来源不同（顶栏会标「实时」）。抓取的状态与停止入口都在「日志」工具里那条采集状态条上，不放在顶栏 —— 抓取是这台设备的事。

### 前置条件

1. 手机打开「开发者选项 → USB 调试」，用数据线连电脑，在手机的「允许 USB 调试吗？」弹窗里点**允许**（勾选「一律允许」更省事）。
2. 本机能执行 `adb devices -l` 并看到设备状态为 `device`。adb 的查找顺序：
   - 环境变量 `ADB_PATH`（直接指向 adb 可执行文件）
   - `ANDROID_HOME` / `ANDROID_SDK_ROOT` 下的 `platform-tools/adb`
   - `PATH` 里的 `adb`
   - 常见安装位置：macOS `~/Library/Android/sdk/platform-tools/adb`、`/opt/homebrew/bin/adb`；Windows `%LOCALAPPDATA%\Android\Sdk\platform-tools\adb.exe`
3. 没装 adb 时面板会直接给出安装提示，不会静默失败。装好 Android SDK Platform-Tools 后重启客户端即可。

> 无线调试：先在终端 `adb pair` / `adb connect <ip>:<端口>` 把设备接进来，再回面板点「刷新设备」。设备一旦出现在 `adb devices` 里，抓取方式与 USB 完全一致。

### 只有一种抓取方式：实时

设备工作区「日志」工具里只有一个「开始实时抓取」按钮，命令是 `adb logcat -v threadtime [-b …]`（长驻子进程），边操作手机边收日志，随时可停；停止后已收到的日志仍可筛选 / 导出。

**为什么不做「一次性拉取」**：`adb logcat` 启动时本身会先把设备缓冲区里已有的日志回放出来，所以「刚刚发生的问题」也看得到。多一个一次性模式只是多一个会静默冻结的按钮，用户看到的现象却是「程序不更新了」。

### 抓取状态：一条留在设备里的状态条

抓取是这台设备的一件事，所以它的状态和停止入口都在「日志」工具里，**不在顶栏**：

```
● 实时抓取中   3,204 条   14 条/秒   03:41   缓冲区 main + system        [停止抓取]
```

- **绿点 + 状态**：抓取中 / 已停止。绿点是 `live-pulse` 呼吸动画，停不下来那种「看着像在跑其实不动」的歧义就没有了。
- **条 / 秒**：`entryCount` 每秒差分。比「总数」更能一眼判断设备是不是真的在产生日志——总数在涨也可能是回放。
- **已抓时长**：从 `DeviceStreamInfo.startedAt` 算，切到「抓包」再切回来不会重置。
- **缓冲区**：这次用的是什么，避免「为什么没有 system 日志」这种问题要回去翻设置。
- **其他日志**：停过一次再开始会多一份日志，这里给入口切回去，不必关掉当前这份。
- 停止后按钮变「重新开始」（用上次的缓冲区选项），状态条本身不消失——它同时也是这份日志的说明。

采集条只在设备工作区的「日志」工具里出现；本地文件工作区没有它。

选项：

- **缓冲区**：`main`（应用日志）、`system`（系统框架，权限 / ActivityManager）、`crash`（崩溃）、`events`。不选 = adb 默认（main + system + crash）。
- **先清空设备缓冲区**（默认**不勾选**）：勾上后先执行 `adb logcat -c`，复现前清场日志更干净；清空后设备上的旧日志无法恢复，所以默认关掉。

> **同一台设备只能有一个实时抓取会话。** 两个 `adb logcat` 会同时回放并跟随同一个环形缓冲区（日志量翻倍、处理翻倍），而第二个会话勾了「先清空」更会直接清掉第一个会话正在收的数据。所以再次对同一 serial 开始抓取时会被拒绝，并自动切到已经在抓的那个标签 —— 要换缓冲区就先在那里停止，再重新开始。

### 实现要点

- **探测与列表**：`adb version` 判断可用性，`adb devices -l` 解析 `model` / `product` / `transport_id`，并把 `unauthorized`（未授权，需在手机上点允许）、`offline`（插拔或 `adb reconnect`）、`no permissions`（Linux udev）区分成可读状态；非 `device` 状态的设备不允许开始抓取。
- **实时**：`adb logcat` 作为子进程长驻，读线程逐行喂给增量解析器 `IncrementalParser`；因为堆栈续行会在错误行**之后**才到，解析器始终扣住最后一条不交付，等下一行到达才释放，这样实时抓取也不会把堆栈和错误行拆开。读线程按「攒够 16 行 **或** 距上次交付超过 120ms」放行一批——只按行数攒批是错的，日志尾部是延迟敏感而非吞吐敏感（空闲手机约 14 行/秒，128 行一批等于 9 秒才刷新一次）。前端每 800ms 轮询一次新增条目并追加到列表末尾，同时刷新 Tag / 包名候选；列表在实时模式下自动跟随最新一行，用户往上滚则停止跟随并显示「回到最新」。
- **包名反推**：启动前先取一次 `ps` 快照（约 120ms），保证 `adb logcat` 回放的缓冲区也能解析出包名；读线程随后每 3 秒刷新一次（只在有日志到达时触发，空闲设备不跑 `ps`），并同时从 `am_proc_start` / `am_kill` / `Start proc` / `Killing` 这些行里学习 pid → 包名 —— 进程死了、`ps` 查不到时，靠的就是缓冲区里这些行。
- **退出清理**：停止抓取、切换日志源、关闭标签、关闭窗口、会话被新会话挤出时都会 kill 掉 `adb logcat` 子进程；`close_session` 也会一并停流。流自然结束（拔线、adb 退出）时前端也会补一次 `stop_device_stream`，否则后端会留着一条「这台设备已被占用」的记录，导致再也无法重新抓取。
- **孤儿流自愈**：webview 重载（Tauri 热更新 / 崩溃恢复）会让前端内存里的标签归零，而 Rust 进程里的 `streams` 还在，于是出现「后端在抓、界面上一个标签都没有」的死结 —— 报错还让人去找那个不存在的标签。现在每条流带一个轮询租约（前端每 800ms 刷一次，`drain()` 顺手续租）：超过 5 秒没人轮询即判定为孤儿，下次点开始时 `reap_orphan_streams` 自动 kill 子进程并清掉会话；若重载后立刻重试（租约未到期），前端在确认本地无该设备标签后会调 `release_device_stream` 强制释放并重试一次。两条路径都只作用于「确实没有标签持有」的流，真正活跃的抓取不受影响。
- **实时抓取为什么不用 `lines()`**：`BufRead::lines()` 要求每行都是合法 UTF-8，遇到第一个坏字节就返回 `stream did not contain valid UTF-8`，整条实时流随之终止。而 App 往 logcat 写 GBK 中文、protobuf 二进制、`events` 二进制 payload 都很常见，一行坏字节就毁掉一次抓取并不合理。现在改为 `read_until(b'\n')` + `String::from_utf8_lossy`：坏字节变成 `�`（U+FFFD），正文照常解析，行尾 CR/LF 手工剥掉（`lines()` 以前顺手做了这件事）。stderr 也一样处理——读不完 stderr 会让子进程卡在写满的管道上。
- **数据不出本机**：全部通过本机 adb 读取，没有网络上传。

### 设备网络请求抓包（设备工作区「抓包」）

> adb 本身没有「网络请求」这层语义，只有 logcat 文本、shell 命令和端口转发。所有抓包方案都是这三样的组合。这一节的能力全部建立在**不修改被测 App** 的前提上。

面板打开时先做一次**只读环境体检**（`getprop` / `su -c id` / `dumpsys package` / 全部代理相关 `settings` 键 / `/proc/net/unix`），结果只用来决定两条采集通道（H5 / 原生）能不能开；界面只呈现「支持什么、不支持什么」，下面四条路线属于技术背景：

| 路线 | HTTPS 明文 | 覆盖面 | 前提 |
| --- | --- | --- | --- |
| `cdp` WebView 调试通道 | ✅ | WebView 内的 H5 / 混合页面 | 设备上有 `webview_devtools_remote_<pid>` socket |
| `mitm-root` MITM + 系统证书 | ✅ | 全设备，含原生 OkHttp | 可 root / 非生产构建 |
| `mitm-debug` MITM + 用户证书 | ⚠️ 条件成立才有 | 取决于 APK 是否声明 `debug-overrides` | 目标包 debuggable |
| `proxy-metadata` 代理元数据 | ❌ | 明文 HTTP 全文 + HTTPS 的 CONNECT 域名、TLS SNI、耗时、字节数、失败原因 | 无条件，**已内建** |

**为什么 CDP 是首选**：Android 允许 debuggable App 在 `user` 构建上保留 WebView 调试，此时 `adb forward tcp:<port> localabstract:webview_devtools_remote_<pid>` + `Network.enable` 就能读到 WebView 里 HTTPS 的**明文 URL、请求头、请求体、响应头、状态码和耗时**——不需要 root、不需要装 CA、不需要改 App。实现是零新增依赖的手写 HTTP/1.1 + WebSocket 客户端（`cdp.rs`）；注意 Android 的 devtools HTTP 服务**会静默丢弃 HTTP/1.0 请求**，必须用 1.1。

局限：CDP 只看得到 WebView 发出的请求，原生接口走不到这个通道；响应体需要额外的 `Network.getResponseBody` 请求，当前版本只采集响应头与大小。App 没打开 H5 页面时 `/json/list` 为空：开启 H5 采集后若一直没有请求，面板会提示先进页面。

#### 原生接口：内建 HTTP/CONNECT 代理

点「开始原生接口采集」会在本机起一个零新增依赖的代理（默认 `18999` 端口，绑定本机 LAN IP），并临时把设备全局代理指向它。采集期间同一个按钮变为「停止原生采集」，H5 也同理。**H5 采集与原生采集可以同时开**，两个通道的请求按 `timeKey` 合并到同一张表，「来源」列区分 `H5` / `原生` / `文件`，方便和 logcat 时间轴对照。请求表随时可以「清空」：清掉当前列表和采集端的待取缓冲，采集线程与设备代理不受影响，之后只显示新请求。

请求表上方是一条**时间轴**：每条请求按 `timeKey` 排在轴上（H5 绿、原生蓝、导入文件灰），轴的跨度是当前全部请求的首尾；**拖拽轴面即可圈出时间区间**，表格只保留落在区间内的请求（显示「命中 / 总数」），点一下轴面空白处或「清除区间」恢复。「跟随最新」打开时表格自动停在底部，向上翻看历史时自动暂停，滚回底部自动恢复；圈选区间时也会暂停，避免被新请求拽走。

时间轴下方是**筛选栏**：关键字匹配 URL / 域名 / 方法（粘贴完整 URL 或只输域名都行），可再叠加方法、状态段（2xx / 3xx / 4xx / 5xx / 失败）与「仅慢请求 ≥ 1s」；筛选与时间轴区间同时生效，互不覆盖，点「清除筛选」一起复位。筛选结果为空时表格提示「没有符合筛选条件的请求」。

| 流量 | 能拿到什么 |
| --- | --- |
| 明文 HTTP | 完整请求行、请求头、请求体、响应头、响应体、状态码、耗时、大小 |
| HTTPS | `CONNECT` 目标域名、TLS ClientHello 里的 **SNI**、连接耗时、上下行字节数、失败原因；隧道原样透传，**不解密** |

**为什么不解密原生 HTTPS**：Android 7+（targetSdk ≥ 24）默认不信任用户安装的 CA；真机上目标包的 `network_security_config.xml` 只有 `<base-config cleartextTrafficPermitted="true"/>`，**没有 `<debug-overrides>` 放行用户 CA**，且 dex 里能查到 `CertificatePinner` 调用痕迹。设备是 `user` 构建、无 `su`、`adb root` 被拒、系统 CA 位于只读的 APEX conscrypt 内——在「不改 App、不 root、不装 CA」的前提下 MITM 走不通，所以这里明确只做元数据，不把「看见域名」伪装成「看见明文」。反过来，`cleartextTrafficPermitted=true` 让明文 HTTP 能拿到完整内容。

- **代理安全**：绑定本机 LAN IP，只放行配对设备 IP、本机 LAN IP 与回环地址，同网段其他人借不了道；停止采集 / 关闭面板 / 退出应用（`ProxyGuard::drop`）都会把设备代理**统一清空为直连**，不写回设置前的值——那个值经常是上一次运行留下的死代理（本机 LAN IP 会变），写回后设备会继续连一个已关闭的端口，看起来就像「停止后代理没清掉」。同一设备只允许一个原生采集（全局代理只有一个），H5 采集可与它并行。
- **代理只影响新进程**：如果被测 App 在「开始原生接口采集」之前已经在运行，它的进程内 `http.proxyHost` 是启动时从 Zygote 继承的，运行中改 `settings` 不会刷新；已有 keep-alive 连接也会继续复用并绕过代理。表现就是**代理已设置、面板在采集、App 继续请求，但表里一条都没有**。开始采集后面板会常驻提示：请完全退出并重新打开被测 App，再触发请求。设置代理时已同步清掉所有旧键、写入 `:0` 触发刷新，并设置 `http_proxy` 与 Android 15 拆分键两份，确保新进程拿到正确代理。
- **AOSP 拆键陷阱**：`settings put global http_proxy <host>:<port>` 在真机上会被拆成 `global_http_proxy_host` / `global_http_proxy_port` 存储，只 `settings delete global http_proxy` **清不干净**，会残留一个指向已停监听的死代理，让设备断网。因此停止时把 `http_proxy`、`global_http_proxy_host`、`global_http_proxy_port`、`global_http_proxy_exclusion_list`、`global_proxy_pac_url` 全部删除后统一写入 `:0`，体检也读全套键。
- **「看着干净」不等于「真的清了」（Android 15 实测）**：`settings delete global http_proxy` 之后 Settings 里读不到任何代理，但 `ConnectivityService` 持有的 Java 层 `http.proxyHost`（由 Zygote 继承）可能仍下发给**新启动**的 App 进程，表现为 `failed to connect to /172.21.33.28 (port 18999)`；force-stop 后重开 App 也一样。因此还原脚本删完所有键后会**显式写入 Android 标准无代理值 `settings put global http_proxy :0`**，触发真正的刷新，`:0` 也按「无代理」解析。**验证是否恢复要看 App 的实际请求日志，不能只看 `settings` 无输出。**
- **崩溃安全（关键）**：设备全局代理是**持久设置**，桌面进程却是易失的——Cmd+Q、强杀、崩溃、先拔线再退出都可能跳过全部析构函数，设备会一直指向已停监听的端口，表现为 `failed to connect to /172.21.33.28 (port 18999)`。所以每次设置成功后都会把「设备 serial + adb 路径」写入 `~/Library/Application Support/com.zaze.loglens/proxy-state.json`（Windows 为 `%APPDATA%\com.zaze.loglens\`），清空成功才删除；**应用启动和每次环境体检都会读这份记录自动清理残留**（面板会显示「已自动修复设备代理」）。设备离线时保留记录，等重新连上再清；清理是无条件的，设备上无论留着谁写的代理都会被清空为直连。
- **手动兜底**：即使内存里的 guard 已随上次进程消失，面板「清空」也会无条件把设备代理清空为直连。
- 部分厂商 ROM 忽略 `settings put global http_proxy`，设置后会读回校验并明确报错，而不是假装成功。

#### 导入第三方工具结果

`导入 HAR / JSONL` 支持 HAR 1.2（Charles / Fiddler / DevTools / mitmproxy `har_dump.py`）和一行一个请求的 JSONL。仓库自带 `scripts/mitmproxy_jsonl.py`：

```bash
LOGLENS_JSONL=~/capture.jsonl mitmdump -s scripts/mitmproxy_jsonl.py -p 8080
# 在面板「外部抓包工具」里填 <本机IP>:8080 → 复现问题 → 导入 capture.jsonl
```

需要原生 HTTPS 明文时只能走外部 MITM（可 root 装系统证书，或目标 APK 带 `debug-overrides`）；这一入口与内建原生采集互斥，避免两个代理互相打架。

**复制 cURL**：选中任一请求，详情右上角「复制 cURL」会生成可直接粘贴到终端的命令（方法、URL、请求头、请求体）。自动跳过 `Host` / `Content-Length` / `Transfer-Encoding` / `Connection` / `Accept-Encoding` 这些由 curl 自行推导的字段；`CONNECT` 行（未解密的 HTTPS 隧道）不生成命令。请求头按原文复制，含真实 token / Cookie，粘贴到终端前请确认目标环境可信。

- **Android 7+ 的硬门槛**：targetSdk ≥ 24 的 App 默认不信任用户安装的 CA，`DEBUGGABLE` 本身**不**等于放行；有证书固定时即使放行也失败。所以体检不会把 `mitm-debug` 标成「可看明文」。
- **凭据按原文保存**：`network.rs::REDACT_ENABLED` 默认 `false`，token / Cookie / 密码等不做脱敏，原文会进入会话、缓存与导出文件——仅在本地受信任环境使用；改成 `true` 即在导入 / 采集落地时替换敏感字段（含 URL query 的 token）。
- **清理**：CDP 的 `adb forward` 由采集线程在停止 / 异常 / 窗口关闭时移除；内建代理停止时同步关闭监听并清空设备代理，不会留下占用端口或死代理。

---

## 7. 多标签：同时分析多个日志

顶栏下方是标签条。**打开文件、连接设备都不会再顶掉当前内容**，而是各开一个新标签。

| 行为 | 结果 |
| --- | --- |
| 打开日志文件 | 新建标签；同一路径已经打开就切过去，不重复占用会话 |
| 开始实时抓取 | 抓到的日志就是设备的「日志」工具的内容并自动切过去；停止后这份日志仍是静态日志，可筛选 / 导出。同一台设备同时只允许一个实时流，重复开始会切到已经在抓的那份 |
| 切走 | 该标签的筛选、排序、选中行、滚动位置全部记住 |
| 切走时的实时流 | **继续接收**，只是不刷新界面；切回来立刻补齐 |
| 关闭标签 | 停掉该标签的 adb 子进程并释放会话；关掉当前标签自动切到相邻标签 |

上限 **4 个标签**，与 Rust 侧的 `MAX_OPEN_SESSIONS` 一致 —— 再多的话 Rust 会静默挤掉最旧的会话，界面上就变成一个「怎么点都查不出数据」的标签。达到上限时直接提示先关一个。

**后台标签怎么继续接收**：每 800ms 一次轮询覆盖所有还带实时流的标签 —— 当前标签走完整流程（拉增量、追加到表格、刷新 Tag / 包名候选），后台标签只把数据从 adb 缓冲区搬进后端会话，不碰界面。这样后台不会因为没人取数据而让缓冲区无限膨胀；切回来时 `catchUp()` 再按页补齐（最多 12 页 / 4800 条，其余滑动时加载）。

**状态怎么隔离**：每个标签在后端是一个独立会话（`sessionId` 就是标签 id），前端是一份 `SessionTab`。切换时先把当前视图状态快照进离开的标签，再恢复目标标签的快照；所有异步响应都带 `requestSeq` 校验，切走之后才返回的旧响应会被丢弃，不会串到新标签里。

---

## 8. 导入日志文件：进度与取消

打开或拖入日志文件后，右下角会出现一张导入卡片：文件名、按字节的百分比、`已读 / 总大小`、已解析条数，以及「取消」。

| 设计点 | 为什么 |
| --- | --- |
| 命令改成 **async + `spawn_blocking`** | Tauri 的同步命令默认在主线程上执行，几十万行的解析会把整个窗口冻住 —— 这才是「什么反应都没有」的真正原因，进度条只是它的表面 |
| **按字节**算百分比 | 行数要解析完才知道总数，字节可以从 `read_until` 的返回值累加，和 `fs::metadata` 的文件大小同量纲 |
| 分 **`parsing` / `indexing`** 两段 | 读完文件后还要建一遍级别 / Tag / pid / 包名候选列表（也是一次全量遍历）。不分段的话进度条会卡在 100% 无缘无故不动 |
| 解析侧每 4096 行回调、事件**节流 100ms** | 够顺滑，又不至于让 JSON 序列化比解析本身还累 |
| **小于 250ms 不显示**卡片 | 一闪而过的进度条看起来像故障，而不是进度 |
| 卡片是**右下角浮层，不是模态** | 大文件导入要花时间，盖住窗口等于换了个样子的「卡住」；浮层下可以继续切标签、翻日志、开始设备抓取 |
| **可取消** | 取消标志是一个 `AtomicBool`，解析线程每块检查一次；命中就丢弃已解析内容，不注册半个会话。取消后标签数不变 |

进度事件名 `log-open-progress`，载荷 `{ requestId, bytesRead, totalBytes, entries, phase }`。`requestId` 由前端生成并在每个事件里回传，多个导入同时在飞时不会互相污染数字。导入期间「打开日志文件」按钮置灰，避免同时开两个。

> **开发模式会慢很多**：29 MB / 30 万行在 release 构建下约 1.0 秒，在 `cargo run` 的 dev profile 下约 12 秒（regex 未优化）。本地点测时进度条走得「慢而稳」是构建配置问题，不是实现问题。

---

## 9. 搜索与筛选：两种模式

搜索框左边是一个两段开关，决定同一个关键字怎么用：

| 模式 | 行为 | 对结果集的影响 |
| --- | --- | --- |
| **筛选**（默认） | 关键字参与 `compute_matches`，匹配整行原文（含堆栈、时间戳、Tag） | 只显示匹配的行，命中数随之变化；导出的也是这批 |
| **查找** | 关键字不参与过滤，改用 `search_hits` 只报告命中位置 | 完全不变；行内高亮 + `3 / 27` 计数，Enter / Shift+Enter 或 ↑↓ 跳转 |

为什么必须分开：想「只看匹配的」时是**筛**，想「在这些里找」时是**查**。合成一种行为有两个具体后果 —— 找一个词会把前后上下文一起藏掉；「清空筛选」会顺手把关键字也清掉（查找不属于筛选，所以清空筛选只清维度，关键字和查找模式保留）。

实现要点：

- `FilterSpec.searchMode`（`filter` / `find`）是唯一开关。后端 `QuickFilter::compile` 在 `find` 模式下**根本不编译搜索条件**，所以 `query_logs` 与 `export_filtered` 天然不受查找影响，不用在导出路径里写特例。
- `search_hits` 复用同一份命中集缓存，只扫描这些条目。匹配字段是 `message`（表格渲染的正文，含堆栈续行）而不是 `raw`：否则命中可能落在时间戳 / Tag 前缀里，行内高亮显示不出来，变成「27 条命中却一条都看不到」。
- 查找不会污染缓存：构造缓存键时把 `find` 模式的关键字清空，否则每敲一个字都会多出一份匹配列表（12 组缓存立刻被冲爆）。
- 跳转要先把目标行所在的分页取回来（`ensureIndexLoaded`）—— 虚拟列表只渲染已加载的页，命中可能在第 40 页。
- 结果集一变（换筛选、换规则、实时抓取追加）命中位置就过期：会重算，但**不移动光标**。自动跳转只在用户输入时发生，否则实时抓取会把视线每 800ms 拽回第一条命中。

---

## 10. Logcat 解析规则

支持 `adb logcat -v` 的常见输出：

| 格式 | 示例 |
| --- | --- |
| threadtime（真机实际输出，无包名） | `11-12 17:28:02.013  1080  1161 D PowerGuruService: message` |
| threadtime（带包名） | `11-12 10:23:45.123  1234-5678/com.foo D/Tag: message` |
| threadtime + uid | `11-12 10:23:45.123 10123 1234 5678/com.foo D/Tag: message` |
| 带年份 | `2026-11-12 10:23:45.123 1234-5678/com.foo W/Tag: message` |
| time | `11-12 10:23:45.123 D/Tag( 1234): message` |
| brief / tag | `D/Tag( 1234): message`、`D/Tag: message` |
| process | `D( 1234) message (Tag)` |
| long | `[ 11-12 10:23:45.123  1234:5678 D/Tag ]` + 缩进行正文 |

> `adb logcat -v threadtime` 在真机上输出的是 **`PID TID L Tag: message`**（没有包名列、级别后面是空格不是斜杠），带 `pid-tid/package` 的是另一类格式。两种都支持；无包名那两条规则会先匹配，避免「消息里恰好有 `E/http: 500` 这类片段」被误当成一条新日志的 tag。

### 包名维度：从 pid 反推

真机 threadtime 没有包名列，但 **pid 一直在**，所以包名是可以还原的，不需要改 logcat 格式：

| 来源 | 形态 | 覆盖场景 |
| --- | --- | --- |
| `adb shell ps -A -o PID,NAME` | `13800 com.example.sample.student` | 抓取开始时取一次快照，之后每 3 秒刷新一次；应用进程的进程名就是它的包名 |
| `events` 缓冲区 | `am_proc_start: [0,13800,10174,com.foo,next-top-activity,{...}]`、`am_kill: [0,13122,com.foo,200,...]` | 进程已经死了还在缓冲区里 —— 崩溃场景正需要 |
| `ActivityManager` 文本行 | `Start proc 13800:com.foo/u0a174 for ...`、`Killing 13122:com.foo/u0a174 (adj 200)` | 默认 `main` 缓冲区里就有，不依赖 `events` |

填充规则（按优先级）：

1. 行内自带包名（`1234-5678/com.foo`）**永远优先**，不被 pid 映射覆盖。
2. 前台应用事件里的包名（`onTopAppChanged: com.example.launcher/...`）描述的是**屏幕上**的应用，也优先于 pid 映射 —— 否则 Launcher 规则会被「这条日志其实是 systemui 打的」这件事破坏。
3. 都没有时，才用 pid → 进程名回填，`com.tencent.mm:push` 这类子进程归并到 `com.tencent.mm`。
4. `system_server`、`logd`、`init`、`vendor.*@*-service` 这些**没有包名**的进程不回填 —— 包名筛选保持「只关于应用」，系统进程的日志仍可用 Tag / PID 找。

结果：包名下拉里只出现真实应用，选中两三个包即可把「我关心的 app」和系统噪音分开。没有 pid 映射可用的行（进程早于缓冲区启动且从未出现在日志里）保持为空，显示 `—`。


设计约定：

- **不丢行**：无法识别的独立行保留为 `?` 级别条目，并在状态栏统计「未识别行」。
- **堆栈归并**：缩进的 `at ...` / `Caused by:` / `... N more` / JSON 正文等续行合并到上一条，命中异常时能看到完整堆栈；行尾显示 `+N 行`，详情抽屉展示原文。
- **缓冲区标记**：`--------- beginning of crash` 生成独立的 `S/logcat` 条目，不污染上一条消息。
- **时间范围**：logcat 无年份，时间统一归一化为 `HH:MM:SS.mmm` 后比较，筛选填 `10:20:00` ~ `10:23:00` 即可；跨年日志请自行分段。
- 无效 UTF-8 按替换字符处理，不会中断解析 —— 文件解析和实时抓取走同一条约定（见下文「实时抓取为什么不用 `lines()`」）。
- **辅助功能前台事件结构化**：`I/AccessibilityData( 2108): onTopAppChanged: <packageName>/<className>` 这类 `brief` / `tag` 格式没有包名列。解析时把 `/` 前的 `AccessibilityEvent.packageName` 填入 `LogEntry.package`（同时进入包名候选），`/` 后的 `AccessibilityEvent.className` 填入 `LogEntry.component`。className 不限 Activity，也可以是 `android.widget.FrameLayout` 之类的弹窗 / 窗口组件，因此规则可以分别按包名和组件类名筛选。普通日志按原格式填充，`component` 为空。

---

## 11. 规则与模板

### 规则模型

一条规则 = 一组条件（`AND` / `OR`），每个条件为 `字段 + 操作符 + 值 + 大小写 + 取反`：

| 字段 | 说明 |
| --- | --- |
| `message` / `raw` | 消息正文（含合并的续行）/ 完整原始文本 |
| `tag` / `package` / `level` | Tag / 应用包名（行内包名列、前台事件包名，或由 pid 反推）/ 级别 |
| `component` | 前台组件类名（`AccessibilityEvent.className`），如 Activity、`android.widget.FrameLayout` |
| `pid` / `tid` | 进程 / 线程号 |
| `timestamp` / `timeKey` | 原始时间 / 归一化时间 |

操作符：包含、不包含、等于、不等于、开头是、结尾是、正则匹配、属于（逗号分隔）、大于等于、小于等于。正则使用 Rust `regex` 语法，默认忽略大小写；非法正则会在界面上直接报错。

快捷筛选（包名 / 级别 / Tag / PID / 搜索 / 排除 / 时间）与规则组可用 `AND`（交集）或 `OR`（并集）组合。

### 内置模板（10 个）

错误与致命、崩溃与异常堆栈、ANR 与卡顿、网络请求、页面生命周期、性能与内存、去掉系统噪音、权限与安全，以及 **Launcher 分组下的「前台应用 / 页面变更」「弹窗 / 非 Activity 组件」**。模板与自定义规则结构完全一致，点「另存」即可改成自己的规则。

> **为什么没有「指定应用包名」模板**：按包名筛有专门的控件——顶部「+ 添加包名」下拉列的是**当前日志里真实出现过的包**，选几个就是多选 chip，不需要手打，也不存在「模板里写死一个 `com.example` 导致点了零命中」的问题。一个内置模板必须**点开即有效**，需要用户先改值的模板不适合放在模板列表里。

**前台应用事件的数据规律**（来自 Android 辅助功能 `AccessibilityEvent.packageName` / `className`）：`-v brief` 日志中 `onTopAppChanged:` 后面是 `<包名>/<组件类名>`，斜杠前是**被观测的应用包名**，斜杠后是**前台页面、弹窗等组件类名**，各种应用、各种组件都可能出现：

```text
11-03 18:02:29.792 I/AccessibilityData( 2108): onTopAppChanged: com.example.launcher/com.example.quickstep.RecentsActivity
11-03 18:02:29.830 I/AccessibilityData( 2108): onTopAppChanged: com.example.sample.launch/com.example.sample.launch.ui.main.MainActivity
11-03 18:25:05.005 I/AccessibilityData( 2108): onTopAppChanged: com.example.sample.web/android.widget.FrameLayout
```

解析后 `package` = `com.example.sample.web`、`component` = `android.widget.FrameLayout`。因此：

| Launcher 分组模板 | 条件 | 用途 |
| --- | --- | --- |
| 前台应用 / 页面变更 | `message` 正则 `(?i)onTopAppChanged` | 抓全部前台切换，不写死 tag / 包名 / 组件名 |
| 弹窗 / 非 Activity 组件 | 上面 + `component` 正则匹配 `android.widget.` / `Dialog` / `Popup` / `Toast` / `Window` | 只看弹窗 / 窗口类组件触发的变更 |

后续要按某个应用建专有规则时，直接在编辑器里用 `package` 字段（等于、包含或正则）配 `onTopAppChanged` 即可，无需改解析器。

### 规则分组与搜索

侧栏按**分组**组织全部规则（内置模板与自定义规则混排，用「模板」小标签区分）：

分组**只做分类**，不绑定任何应用包名——可以按应用、业务模块、问题类型自由划分，同一个应用也可以拆到多个分组里。

- **内置分组**：通用速查 / 稳定性排查 / 功能链路 / Launcher，只读，用来收纳 10 个内置模板。内置分组定义不可改，但**可以往里新增自定义规则**——从分组头 `+` 建的规则会自动归入该分组。
- **自定义分组**：点「+ 分组」创建，只需名称、颜色和说明。分组可改名、改色、删除；删除分组**不会删规则**，规则回到「未分组」。
- **在分组内建规则**：点分组头的 `+`，新建规则会自动归到该分组；规则编辑器里也能随时切换所属分组，或直接「+ 分组」新建一个（保存后自动选中）。
- **搜索**：顶部搜索框支持空格分隔的多关键词（AND），匹配规则名、说明、分组名、条件字段 / 操作符 / 匹配值，以及「内置 / 模板 / 自定义」这类来源词。搜索时自动展开所有命中分组并隐藏空分组，底部显示 `命中 N / 总数`；清空即恢复折叠状态。
- **排序**：内置分组 → 自定义分组（按名称）→ 未分组。

### 持久化位置

| 系统 | 路径 |
| --- | --- |
| macOS | `~/Library/Application Support/com.zaze.loglens/rules.json` |
| Windows | `%APPDATA%\com.zaze.loglens\rules.json` |

界面左下角显示实际路径；「导入规则 / 导出规则」按 JSON 文件操作，方便团队共享规则集。当前文件格式为 v2：

```json
{
  "version": 2,
  "categories": [
    { "id": "custom-cat-…", "name": "星学伴学生端", "description": "学生端作业链路", "color": "#34d399", "builtin": false }
  ],
  "rules": [ { "id": "custom-…", "categoryId": "custom-cat-…", "group": { "logic": "AND", "conditions": [] } } ]
}
```

只保存自定义分组（内置分组来自 `templates.json`）。读取时兼容 v1 的 `{ version: 1, rules: [...] }` 和旧版导出的纯规则数组；分组被删后残留的 `categoryId` 会自动归零，不会丢规则。

---

## 12. 性能设计

- 文件在 Rust 侧流式解析一次，常驻内存；后续筛选只遍历内存条目，不重复读盘。
- 文件导入在**阻塞线程池**里跑（`spawn_blocking`），主线程不被占用，导入期间界面照常可交互；进度事件 100ms 节流一次。实测 29 MB / 30 万行：release 约 1.0 秒（≈29 万行/秒），dev profile 约 12 秒。
- 命中结果按「筛选条件序列化值」缓存（最多 12 组），翻页 / 排序不重复计算匹配集合；实时抓取追加条目时主动失效缓存。
- 前端虚拟滚动只渲染可见行，每次 IPC 取 400 条，滚动到底自动续取。
- **行高固定 26px** 是虚拟滚动便宜的前提，所以消息不换行：长行靠**横向滚动**看全，而不是撑高行。消息列宽度（`--msg-w`）由当前已加载条目的**最长首行**实测得出（canvas `measureText`，避免 CJK / emoji 下 `ch` 失真），并且是从数据算、不是每行 `max-content` —— 后者会让浏览器按「当前渲染了哪几行」重算列宽，纵向滚动时横向滚动条会跳。表头是滚动容器的兄弟节点，靠 `translateX(-scrollLeft)` 跟随，保证标签始终在对应列上方。
- 切换筛选 / 标签后列宽会重算（条目集合变了），单个标签内只增不减。
- 实时抓取每 800ms 轮询一次新增条目，只取增量页，不重读整个会话；空闲轮询不重算 Tag / 包名候选。后端交付门槛是「16 行或 120ms」，实测设备写入一条标记日志到它出现在列表里 < 1.5 秒。
- 多标签下轮询是**一轮覆盖所有标签**的：当前标签走完整增量流程，后台标签只把缓冲搬到会话（每个标签 800ms 一次轻量 IPC）。切回后台标签时一次性补页，最多 12 页，避免长时间不看后一次切换触发几十次查询。
- 每个标签在后端是一个独立会话，会话上限 4 个（`MAX_OPEN_SESSIONS`），标签上限与此对齐。
- 结果导出按当前排序写出，规则非法时提前返回错误而不写文件。

参考量级：几十 MB、几十万行的文本可流畅筛选；超大文件（数百 MB）主要受内存与首次解析时间限制。

---

## 12b. 客户端诊断日志（排查客户端自身异常与流程）

这里记的是 **LogLens 自己** 的故障和流程快照，不是正在分析的那份日志：

| 类型 | 捕获点 |
| --- | --- |
| Rust 崩溃 | 全局 panic hook，记录 panic 文本、位置与调用栈 |
| 前端未捕获异常 | `window.onerror`、`unhandledrejection`、Vue `errorHandler` |
| 命令失败 | `ipc.ts` 的 `call()` 包装：任何 Tauri 命令返回 `Err` 都记一条（轮询类命令除外，避免拆台时刷屏） |
| 流程快照（`info` / 来源 `flow`） | 启动、文件导入、标签切换、设备流、H5/原生抓包、代理恢复、导出，以及所有 `setNotice` 的用户提示；只记阶段、计数与状态 |

- 落盘位置：与 `rules.json` 同目录的 `error-log.jsonl`（macOS `~/Library/Application Support/com.zaze.loglens/`，Windows `%APPDATA%/com.zaze.loglens/`）。
- 上限 500 条，只留最新的；10 秒内完全相同的记录折叠成一条并累计次数（流程快照折叠时保留最新计数），防止高频轮询撑爆日志。
- 顶栏「诊断日志」入口（**只有故障**会产生角标；流程快照不会）：按级别 / 来源筛选，级别选 `流程` 或来源选 `流程` 即可只看全流程；流程行的 detail 直接内联显示新增/累计/可见等计数（不用点开），scope 显示为「采集轮询 / UI 合并 / 设备代理」等阶段名；复制当前筛选，或导出为纯文本贴到 issue。
- **不会**记录日志正文、请求体、Cookie/Token、URL 或文件路径等敏感数据；流程记录只有阶段、计数、状态与用户自选文件名。
- 诊断日志的初始化与写入永不让客户端崩溃；即使落盘失败也会退化到 stderr。遇到「UI 上什么都没出现 / 点了没反应」时，先导出这份日志，按流程快照看最后到达哪个阶段。

---

## 13. 已知限制与后续

- 本地文件仅支持**单个文本文件**，不支持 `.gz` / `.zip` 与多文件合并。
- 同时最多 4 个标签 / 4 个会话。超过时提示先关一个，而不是让 Rust 静默挤掉最旧的会话。
- **同一台设备同时只能有一个实时抓取**（不同设备可以各开一个标签并行抓）。想换缓冲区 / 重新清空，先在采集状态条上点「停止抓取」。无线调试需要先在终端 `adb connect`，面板内暂不做配对。
- 同一份日志不会被重复打开两次：再点同一个文件会切到已有的那个标签。
- 包名靠 pid 反推：进程若在日志缓冲区起点之前就已启动、且从未在日志里留下 `Start proc` / `am_proc_start`，那部分行没有包名（显示 `—`）。离线文件同理，只有日志里出现过的进程才能还原。
- 时间筛选按「时:分:秒」比较，不含日期；跨年日志需分段处理。
- 「查找」只匹配消息正文（含堆栈续行），不匹配时间戳 / PID / Tag；正则模式下只做定位与计数，行内不高亮。
- 行高固定 26px（虚拟滚动的前提），所以消息**不换行**：长行的完整内容靠横向滚动查看，不需要点开详情；但崩溃堆栈这种**多行条目**仍然只在行尾显示 `+N 行`，完整堆栈要点行看右侧抽屉。要「一行内展开全部堆栈」需要改成变高行 + 变高虚拟列表，尚未实现。
- 消息列宽度取「已加载条目的最长首行」，因此只要结果集里有一条 600 字符的行，所有行都会按那个宽度排布（横向滚动条常驻，短行右侧留白）。这是保证列对齐的代价。
- 未做全文索引（如 tantivy），正则筛选大文件仍是一次全量扫描。
- 规划：面板内无线配对、日志来源插件化（文件目录 / 网络）、更多日志类型（iOS、前端埋点、后端 access log）、多文件时间轴对齐、结果对比/差异视图。

---

## 14. 本地验证清单

```bash
# 1) 前端类型检查与构建
npm run build

# 2) Rust 单元测试（解析器、pid→包名、规则引擎、模板 JSON、adb 解析）
cd src-tauri && cargo test --lib && cd ..

# 3) 需要真机的端到端测试（默认跳过）：adb 探测 / 真机 threadtime 解析 / 实时流 / 包名反推
cd src-tauri && cargo test --lib -- --ignored && cd ..

# 4) 桌面端启动
npm run tauri dev
```

手工验收：

1. 打开 `samples/logcat-sample.txt`，应看到 50+ 行被解析成 40+ 条，含 1 条 `?` 未识别行与 5 条 `AccessibilityData` 前台应用事件。
2. 点「崩溃与异常堆栈」，应命中崩溃段（含 `+N 行` 标记），详情抽屉能看到完整堆栈。
3. 点「去掉系统噪音」，Choreographer / OpenGLRenderer 条目应消失。
4. 搜索 `paperId` 并开启正则 `(paperId|totalScore)`，命中数应变化；切换到「错误与致命」后检查级别 chips 联动。
5. 新建一条自定义规则（tag 正则 `^OkHttp$`），测试命中数 → 保存 → 重启客户端后规则仍在。
6. 导出命中结果为 .log，内容条数应与状态栏「命中」一致；导出/导入规则 JSON 往返后规则不丢。
7. 新建分组「星学伴学生端」（只填名称/颜色，无需包名），点分组头 `+` 新建规则 → 保存后规则出现在该分组；改名/改色立即生效，删除分组后规则不丢（回到「未分组」）。
8. 在侧栏搜索框输入分组名、规则名或某个条件匹配值，命中分组自动展开、空分组隐藏；清空搜索后折叠状态恢复。删除自定义分组后规则不丢，出现在「未分组」。
9. 点 Launcher 分组下的「前台应用 / 页面变更」，应命中示例文件末尾的 5 条 `onTopAppChanged` 记录；详情抽屉的「包名 / 组件类名」应分别显示 `com.example.sample.web` 与 `android.widget.FrameLayout`（那条弹窗事件）。
10. 点「弹窗 / 非 Activity 组件」应只命中 1 条（`android.widget.FrameLayout`）；用规则编辑器新建 `package` = `com.example.sample.launch` + `message` 正则 `onTopAppChanged` 的规则，应只命中 1 条。
11. 顶栏「设备」打开选择器，能看到 adb 版本与路径、各设备授权状态与「已打开」标记。把手机 USB 调试关掉再刷新，对应设备应变成「未授权」且点不动（只提示先授权）；已经打开的设备工作区里显示「已断开」，右上角「换设备」可改选。
12. 点「开始实时抓取」：日志工具顶部出现采集状态条（绿点 +「实时抓取中」+ 条数 + 条/秒 + 已抓时长 + 缓冲区），行数 / 条数不为 0（设备缓冲区里已有的日志会先回放出来），Tag 与包名下拉里有候选；点任一模板能命中。
13. 列表「包名」列应有大量应用包名（如 `com.example.sample.student`、`com.example.launcher`），而不是一片 `—`；在筛选栏「+ 添加包名」选中其中一个，只剩该应用的行，再选第二个应变多（多选取并集）。
14. 在手机上随便点几下，1 秒内列表底部出现新日志（列表自动跟随最新行），状态条上的条数与「条/秒」同步变化；向上滚动后不再自动跟随并出现「↓ 回到最新」；点状态条右侧「停止抓取」后绿点变成「已停止」，新日志不再追加，已有日志仍可筛选与导出，按钮变成「重新开始」。
15. 「先清空设备缓冲区」默认不勾选；勾选后开始抓取，行数应从 0 起算（缓冲区内旧日志已清掉）。
16. 实时抓取期间直接关闭窗口，然后 `ps aux | grep "adb.*logcat"` 不应残留 `adb logcat` 进程。

> 非 UTF-8：让被测 App 打一条 GBK 中文，或在本机执行 `adb shell log -p i -t GBK "$(printf '\xd6\xd0\xce\xc4')"`，实时抓取应继续收新日志，坏字节显示为 `�`，日志区不再出现 `stream did not contain valid UTF-8`。

多标签：

17. 开着实时抓取时再「打开日志文件」：标签条出现第二个标签，**实时流没有被中断**——切回设备标签，条数比切走时更多，说明后台一直在收；采集状态条只在设备工作区的「日志」工具里出现，本地文件工作区没有它。
18. 在两个标签上分别设置不同的筛选（比如一个按包名、一个按级别）和滚动位置，来回切换：筛选、行、滚动位置各自保持，不会串。
19. 点标签上的 `×` 关闭当前标签：自动切到相邻标签，被关掉的那个 adb 子进程应结束（`ps aux | grep "adb.*logcat"` 少一个）；关掉全部标签后回到「尚未载入日志」空态。
20. 连开 5 个（4 个标签后再打开一个文件）：应提示「最多同时打开 4 个日志」，且已有的 4 个标签数据不受影响。
21. 实时抓取进行中再去「抓日志」页点开始（换缓冲区 / 勾上清空）：应被拒绝并切回正在抓的那份日志，提示说明已在抓取；已有日志不能因此被清空（条数不归零）。先在状态条上「停止抓取」再开始，则会新建一份日志，旧的可从状态条的「其他日志」切回去。

导入进度：

22. 打开一个几十 MB 的日志：右下角出现导入卡片，百分比持续上涨，`已读 / 总大小` 与条数同步增长；**导入期间界面仍可交互**（能切标签、滚动列表、点筛选），不再整窗无响应。
23. 进度条走完后先出现「正在建立索引」，不会卡在 100% 一动不动；完成后自动切到新标签，提示「已在新标签中载入 …」。
24. 导入中点「取消」：按钮变成「取消中…」后卡片消失，标签数量不变、没有新标签、没有红色错误横幅；再打开同一文件能正常导入。
25. 打开一个几 KB 的小日志：不应看到卡片一闪而过（小于 250ms 的导入不显示卡片）。
26. 导入期间顶栏「打开日志文件」置灰显示「导入中…」，拖入第二个文件时提示「正在导入 …，等它完成再打开下一个」。

长行显示：

27. 打开含超长单行（600+ 字符的 JSON）的日志：消息列自动变宽，表格底部出现横向滚动条；拖到最右端能看到该行结尾，没有被省略号截断；表头随横向滚动同步左移，`级别 / 时间 / PID / TID` 各列仍与数据行对齐。
28. 消息都很短的日志不应出现横向滚动条，消息列仍然填满窗口右侧；切换筛选或标签后列宽按新的结果集重算（从长变短时横向滚动条消失）。

搜索与筛选：

29. 「筛选」模式下输入关键字：列表只剩匹配的行，状态栏「命中」随之减少；切到「查找」：列表恢复成筛选后的全量，出现 `3 / 27` 与 ↑ ↓，Enter 跳下一条、Shift+Enter 上一条，跳到的行有左侧黄色标记并滚动到视口中间。
30. 查找模式下点「清空筛选」：级别 / 包名 / Tag / 时间被清空，但关键字和查找模式保留，计数仍然显示；点「筛选」切回去，同一个关键字立刻收窄列表。
31. 查找正则时列表不变、计数正确；正则写错时搜索框旁显示红色错误而不是整个列表报错。

工作区（顶栏「打开日志文件」/「设备」）：

32. 顶栏右侧只有两个对称入口：「打开日志文件」和「设备」。「设备」点开是选择器（设备列表 + 刷新 + 授权状态），它不代表「当前设备」，只是个创建入口。
33. 点「设备」→ 选一台已授权的设备 → 标签条最左侧出现 `▣ <型号> 设备` 标签，内容区左侧出现竖排工具栏（日志 / 抓包，选中项是蓝块），右侧默认是「日志」工具（还没有日志时显示「开始实时抓取」入口）；**页面里没有任何设备下拉**，换设备只走右上角「换设备」。
34. 同一台设备再选一次：不会开出第二个设备标签，而是聚焦已有的那个。
35. 设备工作区里从左栏「抓包」切到「日志」再切回来：日志面板的筛选 / 滚动位置还在，抓包的请求表也没有被重置——**采集没有中断**（请求表条数继续增长）——**这是本轮的核心验收点**。
36. 设备工作区「日志」工具里点「开始实时抓取」：内容区直接变成日志表格，**顶部出现采集状态条**（绿点 + 实时抓取中 + 条数 + 条/秒 + 已抓时长），**顶栏不再有「停止实时抓取」**，标签条也不会多出一条日志标签；从左栏切到「抓包」再切回「日志」，状态条回来、计时不重置、条数继续增长。
37. 抓取进行中关闭设备工作区（标签上的 × 或页面右上角 ×）：标签条上只剩本地文件标签，**没有残留的日志标签**；`ps aux | grep "adb.*logcat"` 里该会话的进程已结束；`adb shell "settings get global http_proxy"` 返回 `:0`，`lsof -nP -iTCP:18999` 无监听。重新打开该设备时日志是空的（显示「这台设备还没有日志」），代理没有残留。
38. 只分析本地文件（不连设备）：标签条里只有日志标签，设备入口点开后是安装 / 授权提示，两条来源互不干扰；多文件 = 多个日志标签，切换文件就是点标签。

网络请求（设备工作区「抓包」）：

39. 在设备工作区点「抓包」时先做一次只读体检，顶部只有一行环境信息（Android 版本 / 构建类型 / root 状态）和一段能力说明（原生：HTTP 明文全文、HTTPS 不解密；H5：WebView 内 HTTPS 明文）。**界面不展示四条路线的推理与步骤**，那部分只保留在本文档；H5 调试通道缺失时「开始 H5 采集」置灰并说明原因。
40. 点「开始 H5 采集」后，在手机上打开一个 H5 页面：请求表出现 `H5` 来源的行，含方法、状态码、耗时、URL；没打开页面时应提示先进页面，而不是一片空白。
41. 点「开始原生接口采集」：状态行显示代理端点；触发一次明文 HTTP 请求后，表中出现 `原生` 来源的完整请求 / 响应；HTTPS 行显示 `HTTPS 隧道（未解密）` 与 SNI，不算失败。两个开始按钮在运行时会就地变为对应的「停止 … 采集」。
42. H5 与原生采集同时开启时，两类请求按时间混排在同一张表，「来源」列可区分；点任一行能看到请求头 / 响应头，正文按原文显示（脱敏开关默认关闭）。时间轴上的刻度与表格行一一对应，拖拽圈选后表格只剩该区间（显示命中 / 总数），点轴面空白处恢复；「跟随最新」开启时新请求到达后表格停在底部。筛选栏输入域名后只剩该域名的请求，再叠加「仅慢请求 ≥ 1s」与时间区间应同时生效，点「清除筛选」全部复位。
43. 点「停止原生采集」后 `adb shell "settings list global | grep -i proxy"` 应为 `http_proxy=:0`（或无输出），`lsof -nP -iTCP:18999` 无监听；**在平板上重新打开被测 App 触发一次请求，logcat 里不应再出现 `18999` / `ConnectException`**。只看 Settings 无输出不足以证明恢复——Java 层代理属性可能仍在生效。
44. 模拟异常退出：开始原生采集后直接强杀应用（或先拔线再退出），重新打开客户端、或点一次「环境体检」，设备代理应被自动清除，工作区显示「已自动修复设备代理」；`adb shell "settings get global http_proxy"` 返回 `:0`，设备恢复上网，重开被测 App 后请求成功。若设备当时不在线，重新连上后再打开该设备工作区应完成清理，期间不会出现 `failed to connect to /172.21.33.28 (port 18999)`。
45. 选中任一条请求点「复制 cURL」：按钮短暂变「已复制」；粘贴到终端得到的命令含方法、URL、请求头与 `--data-raw` 请求体，`Host` / `Content-Length` / `Connection` / `Accept-Encoding` 不应出现；带 token 的请求复制出来是原值；`CONNECT`（HTTPS 隧道）行没有该按钮。

客户端诊断日志：

46. 顶栏点「诊断日志」：面板显示路径 `…/com.zaze.loglens/error-log.jsonl`。启动流程本身就会写入 `info` 记录，因此正常使用后文件即存在；角标仍只统计故障。
47. 制造一次命令失败（任何会让界面弹出红色错误横幅、即某条 Tauri 命令返回 `Err` 的操作）：关闭并重开「诊断日志」，应看到级别 `error`、来源 `rust`、scope 为对应命令名的记录；同一失败在 10 秒内重复多次只累计 `×N` 而不新增行。
48. 按级别 / 来源 chip 筛选应只保留对应记录；级别选 `流程` 应能看到启动、切标签、打开日志、抓包等快照；有调用栈或 detail 的行可点开看；「复制当前筛选」与「导出」应包含所见记录；「清空」后列表归零且角标消失，重启客户端不会复活。
49. 顶栏角标计数只统计上次打开面板之后的新记录，不会每次启动把旧记录都算成「新」。
