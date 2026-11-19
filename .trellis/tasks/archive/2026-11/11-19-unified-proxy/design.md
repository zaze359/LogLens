# 统一抓包代理 — 技术设计

## 1. 问题定位

两个监听器并存，各自 bind，导致端口漂移（实测证据）：

```
$ lsof -nP -iTCP -sTCP:LISTEN | grep log-analy
log-analy 81346 ... TCP 172.21.32.185:18999 (LISTEN)   # ProxyServer（独立代理）
log-analy 81346 ... TCP 172.21.32.185:52523 (LISTEN)   # ProxyHub（设备采集）

error-log.jsonl:
proxy.listen | 端口占用，改用系统分配端口 | lan=172.21.32.185 preferred=18999 actual=52523
             | error=Address already in use (os error 48)
```

根因是 `AppState` 里存在**两套状态**：

| 路径 | 所有者 | 监听 | 未登记来源 |
|------|--------|------|-----------|
| 设备采集 | `proxy_hub: OnceLock<Arc<ProxyHub>>` | `ProxyHub::new()` → `attach()` | 403 拒绝 |
| 独立代理 | `proxy_server: Mutex<Option<ProxyServer>>` | `ProxyServer` 内部自建 `ProxyHub::with_enroll` | 动态登记 |

## 2. 目标结构

**一个 `ProxyRuntime`（新），承载唯一监听器 + 路由表 + 未绑定来源分组。**

```
AppState
└── proxy_runtime: Mutex<ProxyRuntime>          ← 唯一事实来源
      ├── listener: Option<Listener>            ← 唯一的 LISTEN socket
      ├── routes: Vec<Route>                    ← 已绑定设备（serial + ip）
      └── unnamed: Map<IpAddr, SourceBuffer>    ← 未绑定来源（独立代理页）

DeviceCapture（改造 ProxyCapture）
      ├── serial / ip / offset / shared buffer  ← 绑定时登记的 Route
      └── guard: ProxyGuard                     ← 只负责设备代理配置
```

### 2.1 关键不变量

- **I1 单监听**：`listener` 全进程至多一个；`18999` 只被它占用。
- **I2 常驻**：`listener` 一旦建立，只有显式 `stop_listener` 或进程退出才关闭（PRD Decisions）。
  设备 detach 不再关 socket。
- **I3 配置与传输分离**：`ProxyGuard` 只写设备 `settings`；`Route` 只做流量归属。
  两者拆除互不牵连。
- **I4 分流确定性**：来源 IP 精确匹配 `routes.ip` → 命中设备；否则 → `unnamed`。
  不存在「唯一设备时兜底认领」（原 `route_for` 的 `auto` 分支）——那会把陌生客户端
  混进设备页，与本任务的 R4 冲突。**这是行为变更，需在 implement 中显式回归设备 IP 读不到的场景。**

### 2.2 为什么要去掉 auto 兜底

原 `route_for` 在「只有一个 auto 路由」时认领任意来源，是为了兼容设备 IP 读失败。
统一后所有来源都该有归属（设备 or 未绑定），兜底反而制造歧义。
设备 IP 读失败时，该设备的流量会进入未绑定分组，UI 需明确提示
（`device_ip` 为空 → 提示「未识别设备 IP，请求将显示在独立代理页」）。

**已确认决策**：采用严格分流。宁可让该设备的流量落到独立代理页并明确提示，
也不引入「唯一设备认领全部来源」的兜底——后者会把陌生客户端混进设备页，
与本任务 R4「设备页只显示自己的请求」直接冲突。

## 3. 数据流

```
设备 App / curl
      │ TCP → <lan>:18999
      ▼
run_listener(accept)
      │ 按 peer.ip() 查 routes
      ├─ 命中设备 → Route.shared   → poll_network_capture(id)  → 设备页表格
      └─ 未命中   → unnamed[ip]    → poll_proxy_server()       → 独立代理页（按 IP 分组）
```

轮询仍是 800ms 拉取（前端 `NetworkCaptureTab.vue` / `ProxyServerPanel.vue`），本设计不改轮询模型。

## 4. 接口变更

### 4.1 Rust 内部

| 旧 | 新 |
|----|----|
| `ProxyHub::new()` + `attach(route, lan, port)` | `ProxyRuntime::ensure_listener(lan, port) -> endpoint` |
| `ProxyHub::with_enroll(..)` + `attach_server()` | 删除；统一由 `ProxyRuntime` 的 accept 循环处理未命中来源 |
| `ProxyHub::detach(serial)`（会关 socket） | `ProxyRuntime::detach_device(serial)`（只移路由） |
| `ProxyServer` 整体 | 并入 `ProxyRuntime`，保留 `status()/drain_unnamed()` |

### 4.2 Tauri 命令

保留并调整语义：

- `start_proxy_server(port)` → `ProxyRuntime::ensure_listener` + 标记 `server_mode = true`；
  已存在则复用返回同一 endpoint。
- `stop_proxy_server()` → 若仍有设备绑定，**拒绝**（返回明确错误）或只清 `server_mode`。
  采用：只清 `server_mode` 与未绑定缓冲，设备绑定存在时监听器不关闭。
- `proxy_server_status()` → 增加 `devices: u32`（已绑定设备数）、`unnamedSources: u32`。
- `poll_proxy_server()` → 只返回未绑定来源的请求。
- `start_proxy_capture(serial)` → `ProxyRuntime::ensure_listener` + `bind_device` + `ProxyGuard::apply`。
- `stop_network_capture(id)` → `detach_device` + `guard.restore()`，**不关监听器**。

### 4.3 前端

- `NetworkCaptureTab.vue` 的 `proxyRunning` 判断改为基于设备绑定状态，不再依赖「采集通道」。
- `ProxyServerPanel.vue` 显示 `devices` 数，文案说明「与设备抓包共用同一代理」。
- 顶栏代理状态不变（读 `store.proxyServer`）。

## 5. 风险与对策

| 风险 | 影响 | 对策 |
|------|------|------|
| 去掉 auto 兜底导致设备 IP 读失败时流量进错页 | 设备页空 | `device_ip` 为空时 UI 明确提示并给出处理建议 |
| `stop_proxy_server` 在设备绑定期间被调 | 设备断网 | 明确语义：不清设备绑定、不关监听器，只退出「独立代理」模式 |
| 常驻监听器被遗忘 | 端口长期占用 | 退出时 `stop_all_streams` 已统一清理；UI 明确显示运行状态 |
| 现有 124 个测试依赖 `ProxyHub::*` | 编译失败 | implement 中先迁移测试辅助函数，再改实现 |

## 6. 兼容性与回滚

- 无持久化格式变更（`proxy-state.json` 不变），回滚只需恢复原文件。
- 保留 `ProxyServerStatus` 字段（新增字段有 `default`），前端旧代码可兼容。
- 回滚点：本设计是纯 Rust/前端源码改动，`git revert` 即可；无数据迁移。

## 7. 实施顺序（详见 implement.md）

1. 抽 `ProxyRuntime`，迁移路由表与 accept 循环，保持设备路径可用。
2. 把 `ProxyServer` 并入，`proxy_server` 命令改走 `ProxyRuntime`。
3. `ProxyCapture` 改为绑定语义，移除 `ProxyHub`。
4. 前端适配与提示。
5. 全量测试 + 真实 curl 端到端 + 构建。