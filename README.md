# LogLens · 日志分析客户端

分析各种采集日志的桌面客户端。首个版本聚焦 **Android Logcat 本地文件**：文件解析放在 Rust 侧，Vue3 负责交互，规则与模板可以快速把海量日志收敛到几条关键线索。

> 目标平台：macOS、Windows（同一套代码，分别在对应系统打包）。技术栈：Tauri 2 + Rust + Vue 3 + TypeScript + Vite。

---

## 1. 本期范围

| 能力 | 状态 |
| --- | --- |
| 日志来源 | ✅ 本地文件（.log / .txt / .logcat）+ 直连 Android 设备（adb 实时抓取） |
| 设备直连 | ✅ adb 探测、设备列表（授权 / 未授权 / 离线）、实时抓取、缓冲区选择、可选先清空缓冲区 |
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
│   ├── components/               # 顶栏、标签条、规则侧栏、分组/条件编辑器、筛选栏、统计、虚拟表格、详情抽屉、设备抓取面板
│   └── styles/main.css           # 深色主题
├── src-tauri/
│   ├── Cargo.toml / build.rs / tauri.conf.json / capabilities/default.json
│   ├── templates.json            # 内置分组 + 快捷模板（纯 JSON，编译期嵌入二进制）
│   └── src/
│       ├── model.rs              # 前后端共享的数据结构（camelCase）
│       ├── logcat.rs             # Logcat 解析器（含流式增量解析）+ 单元测试
│       ├── rules.rs              # 规则编译与匹配 + 单元测试
│       ├── builtin.rs            # 加载模板 + 模板合法性单测
│       ├── adb.rs                # adb 定位、设备枚举、logcat 参数构造
│       ├── stream.rs             # 实时 adb logcat 子进程与读线程
│       ├── session.rs            # 日志会话、分页查询、命中索引缓存、流式追加
│       ├── config_store.rs       # 自定义规则持久化
│       ├── commands.rs           # Tauri IPC 命令
│       └── lib.rs / main.rs      # 应用入口
├── scripts/
│   ├── setup.sh / setup.ps1      # 一键检测 / 安装 / 运行
│   └── gen-icon.cjs              # 生成 Tauri 编译期默认窗口图标（零依赖、幂等）
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
| 打包（检查图标并自动补全 `bundle.icon`） | `--build` | `-Build` |
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

## 5. 打包（macOS / Windows）

**开发态图标（自动生成，无需操作）**：Tauri 2 的 `generate_context!()` 在编译期会读取 `src-tauri/icons/icon.png` 作为默认窗口图标，**即使 `bundle.icon` 是 `[]` 也一样**。因此 `npm install`（`postinstall`）、`npm run dev`、`npm run build`、`npm run tauri dev/build` 以及一键脚本都会执行 `scripts/gen-icon.cjs`，用纯 Node 标准库生成一张 64×64 占位渐变 PNG（幂等，不会覆盖你已有的图标）。新克隆的仓库可以直接启动，不需要提交二进制图标；`npm install` 之后直接执行 `cd src-tauri && cargo test --lib` 也能编译通过。

> 只有**完全没有跑过 npm 脚本**、直接在干净仓库里调用 `cargo build` 才会因为缺 `icon.png` 失败，此时补跑一次 `node scripts/gen-icon.cjs` 即可。

**第一步：生成正式打包图标**（发布时需要）：

```bash
npm run tauri icon path/to/icon.png     # 建议 1024x1024 PNG，生成 src-tauri/icons/*
```

然后填写 `src-tauri/tauri.conf.json` 的 `bundle.icon`：

```json
"icon": [
  "icons/32x32.png",
  "icons/128x128.png",
  "icons/128x128@2x.png",
  "icons/icon.icns",
  "icons/icon.ico"
]
```

> 用一键脚本打包（`setup.sh --build` / `setup.ps1 -Build`）时，脚本发现图标文件齐全会**自动补全**这个数组，无需手动改配置；只有手动执行 `npm run tauri build` 才需要按上面填写。`icon: []` 是开发态默认值，保证新克隆的仓库能直接 `tauri dev`。

**第二步：在目标平台上构建**（Tauri 不做跨系统打包，mac 出 dmg、Windows 出 msi 需要各自系统或 CI）：

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

## 6. 从 Android 设备抓取日志（adb）

顶栏「连接设备」可以直接从已连接的手机读 logcat，不需要先把日志导成文件。抓到的日志与本地文件走**同一套解析器、规则、筛选与导出**，只是来源不同（顶栏会标「实时」）。

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

面板只有一个「开始实时抓取」按钮，命令是 `adb logcat -v threadtime [-b …]`（长驻子进程），边操作手机边收日志，随时可停；停止后已收到的日志仍可筛选 / 导出。

**为什么不做「一次性拉取」**：`adb logcat` 启动时本身会先把设备缓冲区里已有的日志回放出来，所以「刚刚发生的问题」也看得到。多一个一次性模式只是多一个会静默冻结的按钮，用户看到的现象却是「程序不更新了」。

选项：

- **缓冲区**：`main`（应用日志）、`system`（系统框架，权限 / ActivityManager）、`crash`（崩溃）、`events`。不选 = adb 默认（main + system + crash）。
- **先清空设备缓冲区**（默认**不勾选**）：勾上后先执行 `adb logcat -c`，复现前清场日志更干净；清空后设备上的旧日志无法恢复，所以默认关掉。

> **同一台设备只能有一个实时抓取会话。** 两个 `adb logcat` 会同时回放并跟随同一个环形缓冲区（日志量翻倍、处理翻倍），而第二个会话勾了「先清空」更会直接清掉第一个会话正在收的数据。所以再次对同一 serial 开始抓取时会被拒绝，并自动切到已经在抓的那个标签 —— 要换缓冲区就先在那里停止，再重新开始。

### 实现要点

- **探测与列表**：`adb version` 判断可用性，`adb devices -l` 解析 `model` / `product` / `transport_id`，并把 `unauthorized`（未授权，需在手机上点允许）、`offline`（插拔或 `adb reconnect`）、`no permissions`（Linux udev）区分成可读状态；非 `device` 状态的设备不允许开始抓取。
- **实时**：`adb logcat` 作为子进程长驻，读线程逐行喂给增量解析器 `IncrementalParser`；因为堆栈续行会在错误行**之后**才到，解析器始终扣住最后一条不交付，等下一行到达才释放，这样实时抓取也不会把堆栈和错误行拆开。读线程按「攒够 16 行 **或** 距上次交付超过 120ms」放行一批——只按行数攒批是错的，日志尾部是延迟敏感而非吞吐敏感（空闲手机约 14 行/秒，128 行一批等于 9 秒才刷新一次）。前端每 800ms 轮询一次新增条目并追加到列表末尾，同时刷新 Tag / 包名候选；列表在实时模式下自动跟随最新一行，用户往上滚则停止跟随并显示「回到最新」。
- **包名反推**：启动前先取一次 `ps` 快照（约 120ms），保证 `adb logcat` 回放的缓冲区也能解析出包名；读线程随后每 3 秒刷新一次（只在有日志到达时触发，空闲设备不跑 `ps`），并同时从 `am_proc_start` / `am_kill` / `Start proc` / `Killing` 这些行里学习 pid → 包名 —— 进程死了、`ps` 查不到时，靠的就是缓冲区里这些行。
- **退出清理**：停止抓取、切换日志源、关闭标签、关闭窗口、会话被新会话挤出时都会 kill 掉 `adb logcat` 子进程；`close_session` 也会一并停流。流自然结束（拔线、adb 退出）时前端也会补一次 `stop_device_stream`，否则后端会留着一条「这台设备已被占用」的记录，导致再也无法重新抓取。
- **数据不出本机**：全部通过本机 adb 读取，没有网络上传。

---

## 7. 多标签：同时分析多个日志

顶栏下方是标签条。**打开文件、连接设备都不会再顶掉当前内容**，而是各开一个新标签。

| 行为 | 结果 |
| --- | --- |
| 打开日志文件 | 新建标签；同一路径已经打开就切过去，不重复占用会话 |
| 开始实时抓取 | 新建标签；停止后标签保留为静态日志，仍可筛选 / 导出。同一台设备同时只允许一个实时流，重复开始会切到已有标签 |
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
| `adb shell ps -A -o PID,NAME` | `13800 com.hailiang.workcloud.student` | 抓取开始时取一次快照，之后每 3 秒刷新一次；应用进程的进程名就是它的包名 |
| `events` 缓冲区 | `am_proc_start: [0,13800,10174,com.foo,next-top-activity,{...}]`、`am_kill: [0,13122,com.foo,200,...]` | 进程已经死了还在缓冲区里 —— 崩溃场景正需要 |
| `ActivityManager` 文本行 | `Start proc 13800:com.foo/u0a174 for ...`、`Killing 13122:com.foo/u0a174 (adj 200)` | 默认 `main` 缓冲区里就有，不依赖 `events` |

填充规则（按优先级）：

1. 行内自带包名（`1234-5678/com.foo`）**永远优先**，不被 pid 映射覆盖。
2. 前台应用事件里的包名（`onTopAppChanged: com.zui.launcher/...`）描述的是**屏幕上**的应用，也优先于 pid 映射 —— 否则 Launcher 规则会被「这条日志其实是 systemui 打的」这件事破坏。
3. 都没有时，才用 pid → 进程名回填，`com.tencent.mm:push` 这类子进程归并到 `com.tencent.mm`。
4. `system_server`、`logd`、`init`、`vendor.*@*-service` 这些**没有包名**的进程不回填 —— 包名筛选保持「只关于应用」，系统进程的日志仍可用 Tag / PID 找。

结果：包名下拉里只出现真实应用，选中两三个包即可把「我关心的 app」和系统噪音分开。没有 pid 映射可用的行（进程早于缓冲区启动且从未出现在日志里）保持为空，显示 `—`。


设计约定：

- **不丢行**：无法识别的独立行保留为 `?` 级别条目，并在状态栏统计「未识别行」。
- **堆栈归并**：缩进的 `at ...` / `Caused by:` / `... N more` / JSON 正文等续行合并到上一条，命中异常时能看到完整堆栈；行尾显示 `+N 行`，详情抽屉展示原文。
- **缓冲区标记**：`--------- beginning of crash` 生成独立的 `S/logcat` 条目，不污染上一条消息。
- **时间范围**：logcat 无年份，时间统一归一化为 `HH:MM:SS.mmm` 后比较，筛选填 `10:20:00` ~ `10:23:00` 即可；跨年日志请自行分段。
- 无效 UTF-8 按替换字符处理，不会中断整个文件解析。
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
11-03 18:02:29.792 I/AccessibilityData( 2108): onTopAppChanged: com.zui.launcher/com.zui.quickstep.RecentsActivity
11-03 18:02:29.830 I/AccessibilityData( 2108): onTopAppChanged: com.hljzjx.edu.student.launch/com.hljzjx.launch.ui.main.MainActivity
11-03 18:25:05.005 I/AccessibilityData( 2108): onTopAppChanged: com.hailiang.xxb.web.app/android.widget.FrameLayout
```

解析后 `package` = `com.hailiang.xxb.web.app`、`component` = `android.widget.FrameLayout`。因此：

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

## 13. 已知限制与后续

- 本地文件仅支持**单个文本文件**，不支持 `.gz` / `.zip` 与多文件合并。
- 同时最多 4 个标签 / 4 个会话。超过时提示先关一个，而不是让 Rust 静默挤掉最旧的会话。
- **同一台设备同时只能有一个实时抓取**（不同设备可以各开一个标签并行抓）。想换缓冲区 / 重新清空，先在该标签点「停止实时抓取」。无线调试需要先在终端 `adb connect`，面板内暂不做配对。
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
9. 点 Launcher 分组下的「前台应用 / 页面变更」，应命中示例文件末尾的 5 条 `onTopAppChanged` 记录；详情抽屉的「包名 / 组件类名」应分别显示 `com.hailiang.xxb.web.app` 与 `android.widget.FrameLayout`（那条弹窗事件）。
10. 点「弹窗 / 非 Activity 组件」应只命中 1 条（`android.widget.FrameLayout`）；用规则编辑器新建 `package` = `com.hljzjx.edu.student.launch` + `message` 正则 `onTopAppChanged` 的规则，应只命中 1 条。
11. 点顶栏「连接设备」：adb 状态行显示版本与路径，设备下拉里能看到手机型号；把手机 USB 调试关掉再刷新，对应设备应变成「未授权」且「开始实时抓取」不可点。
12. 点「开始实时抓取」：顶栏出现绿点「实时」，状态栏显示「实时抓取中」，行数 / 条数不为 0（设备缓冲区里已有的日志会先回放出来），Tag 与包名下拉里有候选；点任一模板能命中。
13. 列表「包名」列应有大量真实应用包名（如 `com.hailiang.workcloud.student`、`com.zui.launcher`），而不是一片 `—`；在筛选栏「+ 添加包名」选中其中一个，只剩该应用的行，再选第二个应变多（多选取并集）。
14. 在手机上随便点几下，1 秒内列表底部出现新日志（列表自动跟随最新行），行数/条数与 Tag 候选同步增长；向上滚动后不再自动跟随并出现「↓ 回到最新」；点「停止实时抓取」后新日志不再追加，已有日志仍可筛选与导出。
15. 「先清空设备缓冲区」默认不勾选；勾选后开始抓取，行数应从 0 起算（缓冲区内旧日志已清掉）。
16. 实时抓取期间直接关闭窗口，然后 `ps aux | grep "adb.*logcat"` 不应残留 `adb logcat` 进程。

多标签：

17. 开着实时抓取时再「打开日志文件」：标签条出现第二个标签，**实时流没有被中断**——切回实时标签，行数比切走时更多，说明后台一直在收；顶栏「停止实时抓取」只在实时标签上出现。
18. 在两个标签上分别设置不同的筛选（比如一个按包名、一个按级别）和滚动位置，来回切换：筛选、行、滚动位置各自保持，不会串。
19. 点标签上的 `×` 关闭当前标签：自动切到相邻标签，被关掉的那个 adb 子进程应结束（`ps aux | grep "adb.*logcat"` 少一个）；关掉全部标签后回到「尚未载入日志」空态。
20. 连开 5 个（4 个标签后再打开一个文件）：应提示「最多同时打开 4 个日志」，且已有的 4 个标签数据不受影响。
21. 实时抓取进行中再对它「开始实时抓取」（换缓冲区 / 勾上清空）：应被拒绝并自动切到正在抓的那个标签，提示里点名该标签；已有标签的日志不能因此被清空（行数不归零）。先在该标签点「停止实时抓取」再开始，则应正常新建标签。

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
