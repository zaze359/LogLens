# 统一抓包代理 — 执行计划

> 依赖 `design.md` 的 I1–I4 不变量。每步都可独立编译、可回滚。

## 0. 基线（改动前）

```bash
cd src-tauri && cargo test --lib        # 期望 124 passed
cd .. && npm run build                  # 期望通过
```

记录基线，任一步骤导致用例数下降即停下排查。

---

## 1. 引入 `ProxyRuntime`（保持设备路径可用）

**改**：`src-tauri/src/proxy.rs`

- 新增 `ProxyRuntime`：持有 `listener: Option<Listener>`、`routes: Vec<Route>`、`unnamed: HashMap<IpAddr, SourceBuffer>`。
- 迁移 `ensure_listener` / `bind_listener` / `bound_endpoint` / `run_listener` 进 `ProxyRuntime`。
- `accept` 循环：`route_for(routes, ip)` 命中 → 设备 buffer；未命中 → `unnamed.entry(ip)`。
- `ProxyHub` 暂时保留为 `ProxyRuntime` 的薄包装，避免一次性改动过大。

**验证**：`cargo test --lib` 仍 124 passed。

**回滚点**：此步独立，`git checkout src-tauri/src/proxy.rs` 即还原。

---

## 2. 并入 `ProxyServer`

**改**：`proxy.rs`、`session.rs`

- 删除 `ProxyServer` 结构，`AppState.proxy_server` 改为 `proxy_runtime: Mutex<ProxyRuntime>`。
- `ProxyHub` 删除。
- `enroll` 机制删除（由 `unnamed` 分组替代）。

**验证**：编译通过；`cargo test --lib` 全绿（需先迁移测试 helper，见步骤 5）。

---

## 3. `ProxyCapture` 改为绑定语义

**改**：`proxy.rs`、`commands.rs`

- `ProxyCapture::start`：`ensure_listener` → `bind_device(serial, ip, offset, buffer)` → `ProxyGuard::apply`。
- 失败时 `unbind_device(serial)`，但**不关监听器**（I2）。
- `ProxyCapture::stop` / `Drop`：只 `unbind_device` + `guard.restore()`。
- 命令 `start_proxy_capture` / `stop_network_capture` / `close_network_capture` 同步调整。

**验证**：`cargo test --lib` 全绿。

---

## 4. 命令层：独立代理改走 `ProxyRuntime`

**改**：`commands.rs`

- `start_proxy_server(port)`：`ensure_listener` + `server_mode = true`；返回 `endpoint` + `devices` + `unnamedSources`。
- `stop_proxy_server()`：清 `server_mode` 与 `unnamed` 缓冲；**若仍有设备绑定则不关监听器**，返回当前状态而非错误。
- `proxy_server_status()`：新增 `devices` / `unnamedSources` 字段（`ProxyServerStatus` 加 `#[serde(default)]`）。
- `poll_proxy_server()`：只取 `unnamed`。

**验证**：`cargo build --lib`（命令签名变化由 `lib.rs` 编译错误兜底）。

---

## 5. 迁移测试（先于步骤 3，避免中途红）

**改**：`proxy.rs` 的 `mod tests`

- `route(...)` / `unbound_route(...)` helper 改到新 API。
- `several_devices_share_the_same_listener` → 断言 `ensure_listener` 幂等。
- `the_listener_closes_when_the_last_device_leaves` → 改为断言**常驻**（I2 行为变更，测试需反向改）。
- `a_standalone_proxy_serves_any_client_*` / `a_real_curl_client_*` → 改到 `ProxyRuntime` API。

**新增测试**：
- `one_listener_is_shared_by_a_device_and_a_standalone_client`（R1）
- `an_unknown_source_never_lands_in_a_device_buffer`（R4，去掉 auto 兜底）
- `stopping_a_device_keeps_the_listener_and_other_routes`（R3）

**验证**：`cargo test --lib`，用例数 ≥ 124 + 新增数。

---

## 6. 前端适配

**改**：`src/components/ProxyServerPanel.vue`、`src/components/device/NetworkCaptureTab.vue`、`src/types.ts`

- 类型补 `devices` / `unnamedSources`。
- 面板文案：「与设备抓包共用同一代理（18999），此处只显示未绑定设备的请求」。
- 设备页 `device_ip` 为空时提示「未识别设备 IP，请求将显示在独立代理页」（design 决策）。
- 设备页显示共享端点，避免「采集端点」字样让人误以为每台设备一个端口。

**验证**：`npm run build`。

---

## 7. 端到端验收

```bash
# AC1/AC2：单监听 + 未绑定来源被接收
cd src-tauri && cargo test --lib -- a_real_curl_client
cargo test --lib -- one_listener_is_shared
cargo test --lib -- an_unknown_source

# AC5：全量回归
cargo test --lib
cd .. && npm run build
```

真机验证（需用户配合）：
1. 顶栏开启代理 → 记下 endpoint。
2. 设备页开始抓包 → endpoint 必须与上一步**完全相同**，`lsof -nP -iTCP:18999` 只有一个 LISTEN。
3. 设备页只出现该设备流量；curl 指向同一端点，出现在独立代理页。
4. 关闭设备抓包 → 设备代理还原，监听器仍在，curl 仍可抓。

---

## 8. 收尾

- 更新 `.trellis/spec/backend/directory-structure.md`：`proxy.rs` 的职责描述（单一 runtime）。
- 更新 `.trellis/spec/backend/logging-guidelines.md` 如有新增 scope（`proxy.runtime.*`）。
- `python3 ./.trellis/scripts/task.py validate 11-19-unified-proxy`
- 提交前跑 `npm run build` + `cargo test --lib`。