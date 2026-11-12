# LogLens · 日志分析客户端

分析各种采集日志的桌面客户端。首个版本聚焦 **Android Logcat 本地文件**：文件解析放在 Rust 侧，Vue3 负责交互，规则与模板可以快速把海量日志收敛到几条关键线索。

> 目标平台：macOS、Windows（同一套代码，分别在对应系统打包）。技术栈：Tauri 2 + Rust + Vue 3 + TypeScript + Vite。

---

## 1. 本期范围

| 能力 | 状态 |
| --- | --- |
| macOS / Windows 桌面客户端 | ✅ Tauri 2，双平台构建脚本一致 |
| Android Logcat 本地文件分析 | ✅ 单文件、流式解析、多格式兼容 |
| 快捷筛选：级别 / Tag / PID / 关键字 / 正则 / 排除 / 时间范围 / 排序 | ✅ |
| 自定义规则：多条件 AND·OR、字段 & 操作符、取反、正则、大小写 | ✅ 可视化编辑器 + 命中数测试 |
| 内置快捷模板 | ✅ 11 个，归入 4 个分组（通用速查 / 稳定性排查 / 功能链路 / Launcher） |
| 规则分组（分类） | ✅ 一等公民分组，纯分类不绑定应用：可建 / 改 / 删 / 折叠；内置 4 个（通用速查 / 稳定性排查 / 功能链路 / Launcher） |
| 规则搜索 | ✅ 侧栏实时多关键词搜索，覆盖规则名 / 说明 / 分组名 / 条件字段与匹配值 |
| 规则持久化与导入导出 | ✅ JSON v2（分组 + 规则一起），兼容旧的 v1 与纯规则数组 |
| 结果导出 | ✅ 命中日志原样导出为 .log |
| 大文件友好 | ✅ 虚拟滚动 + 后端分页 + 命中索引缓存 |
| adb 实时流 / 多文件合并 / 压缩文件 / 其他日志类型 | ⛔ 规划中（见第 9 节） |

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
│   ├── stores/session.ts         # 全局状态、查询/分页/规则/导入导出动作
│   ├── components/               # 顶栏、规则侧栏、分组编辑器、条件编辑器、筛选栏、统计、虚拟表格、详情抽屉
│   └── styles/main.css           # 深色主题
├── src-tauri/
│   ├── Cargo.toml / build.rs / tauri.conf.json / capabilities/default.json
│   ├── templates.json            # 内置分组 + 快捷模板（纯 JSON，编译期嵌入二进制）
│   └── src/
│       ├── model.rs              # 前后端共享的数据结构（camelCase）
│       ├── logcat.rs             # Logcat 解析器 + 单元测试
│       ├── rules.rs              # 规则编译与匹配 + 单元测试
│       ├── builtin.rs            # 加载模板 + 模板合法性单测
│       ├── session.rs            # 日志会话、分页查询、命中索引缓存
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

## 6. Logcat 解析规则

支持 `adb logcat -v` 的常见输出：

| 格式 | 示例 |
| --- | --- |
| threadtime（默认） | `11-12 10:23:45.123  1234-5678/com.foo D/Tag: message` |
| threadtime + uid | `11-12 10:23:45.123 10123 1234 5678/com.foo D/Tag: message` |
| 带年份 | `2026-11-12 10:23:45.123 1234-5678/com.foo W/Tag: message` |
| time | `11-12 10:23:45.123 D/Tag( 1234): message` |
| brief / tag | `D/Tag( 1234): message`、`D/Tag: message` |
| process | `D( 1234) message (Tag)` |
| long | `[ 11-12 10:23:45.123  1234:5678 D/Tag ]` + 缩进行正文 |

设计约定：

- **不丢行**：无法识别的独立行保留为 `?` 级别条目，并在状态栏统计「未识别行」。
- **堆栈归并**：缩进的 `at ...` / `Caused by:` / `... N more` / JSON 正文等续行合并到上一条，命中异常时能看到完整堆栈；行尾显示 `+N 行`，详情抽屉展示原文。
- **缓冲区标记**：`--------- beginning of crash` 生成独立的 `S/logcat` 条目，不污染上一条消息。
- **时间范围**：logcat 无年份，时间统一归一化为 `HH:MM:SS.mmm` 后比较，筛选填 `10:20:00` ~ `10:23:00` 即可；跨年日志请自行分段。
- 无效 UTF-8 按替换字符处理，不会中断整个文件解析。
- **辅助功能前台事件结构化**：`I/AccessibilityData( 2108): onTopAppChanged: <packageName>/<className>` 这类 `brief` / `tag` 格式没有包名列。解析时把 `/` 前的 `AccessibilityEvent.packageName` 填入 `LogEntry.package`（同时进入包名候选），`/` 后的 `AccessibilityEvent.className` 填入 `LogEntry.component`。className 不限 Activity，也可以是 `android.widget.FrameLayout` 之类的弹窗 / 窗口组件，因此规则可以分别按包名和组件类名筛选。普通日志按原格式填充，`component` 为空。

---

## 7. 规则与模板

### 规则模型

一条规则 = 一组条件（`AND` / `OR`），每个条件为 `字段 + 操作符 + 值 + 大小写 + 取反`：

| 字段 | 说明 |
| --- | --- |
| `message` / `raw` | 消息正文（含合并的续行）/ 完整原始文本 |
| `tag` / `package` / `level` | Tag / 应用包名（`AccessibilityEvent.packageName`）/ 级别 |
| `component` | 前台组件类名（`AccessibilityEvent.className`），如 Activity、`android.widget.FrameLayout` |
| `pid` / `tid` | 进程 / 线程号 |
| `timestamp` / `timeKey` | 原始时间 / 归一化时间 |

操作符：包含、不包含、等于、不等于、开头是、结尾是、正则匹配、属于（逗号分隔）、大于等于、小于等于。正则使用 Rust `regex` 语法，默认忽略大小写；非法正则会在界面上直接报错。

快捷筛选（级别 / Tag / PID / 搜索 / 排除 / 时间）与规则组可用 `AND`（交集）或 `OR`（并集）组合。

### 内置模板（11 个）

错误与致命、崩溃与异常堆栈、ANR 与卡顿、网络请求、页面生命周期、性能与内存、去掉系统噪音、指定应用包名、权限与安全，以及 **Launcher 分组下的「前台应用 / 页面变更」「弹窗 / 非 Activity 组件」**。模板与自定义规则结构完全一致，点「另存」即可改成自己的规则。

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

- **内置分组**：通用速查 / 稳定性排查 / 功能链路 / Launcher，只读，用来收纳 11 个内置模板。内置分组定义不可改，但**可以往里新增自定义规则**——从分组头 `+` 建的规则会自动归入该分组。
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

## 8. 性能设计

- 文件在 Rust 侧流式解析一次，常驻内存；后续筛选只遍历内存条目，不重复读盘。
- 命中结果按「筛选条件序列化值」缓存（最多 12 组），翻页 / 排序不重复计算匹配集合。
- 前端虚拟滚动只渲染可见行，每次 IPC 取 400 条，滚动到底自动续取。
- 结果导出按当前排序写出，规则非法时提前返回错误而不写文件。

参考量级：几十 MB、几十万行的文本可流畅筛选；超大文件（数百 MB）主要受内存与首次解析时间限制。

---

## 9. 已知限制与后续

- 仅支持**单个本地文本文件**，不支持 `.gz` / `.zip`、多文件合并、文件或 adb 实时流。
- 时间筛选按「时:分:秒」比较，不含日期；跨年日志需分段处理。
- 未做全文索引（如 tantivy），正则筛选大文件仍是一次全量扫描。
- 规划：日志来源插件化（adb / 文件目录 / 网络）、更多日志类型（iOS、前端埋点、后端 access log）、多文件时间轴对齐、规则分组与着色、结果对比/差异视图。

---

## 10. 本地验证清单

```bash
# 1) 前端类型检查与构建
npm run build

# 2) Rust 单元测试（解析器、规则引擎、模板 JSON）
cd src-tauri && cargo test --lib && cd ..

# 3) 桌面端启动
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
