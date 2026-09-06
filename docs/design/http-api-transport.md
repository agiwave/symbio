# HTTP(S) 开放接口插件设计（v2：插件化 + WebSocket）

> 目标：在 App 运行期间对外提供一套纯 HTTP/WebSocket 接口，让第三方应用像前端一样访问和控制整个应用。
> 接口服务本身作为**一个普通插件**纳入现有插件体系，配置/开启/关闭**复用现有设置页机制**，前端零改动。

- 状态：设计稿 v2（待评审）
- 日期：2026-09-06
- 上一版：`http-api-transport.md` 的 HTTP transport 方案（已由本版取代：服务从 tauri 壳层下沉为插件，SSE 改为 WebSocket 为主）

---

## 1. 结论（TL;DR）

1. **做成插件完全可行，且是更优解**——设置页的配置 UI 是现成机制，前端真的不用动。
2. **WebSocket 是对的**，它确实是本场景的最优传输。
   但有个必须写进设计的细节：**WebSocket 解决的是"通道"，不解决"信源"**——
   `session/chat/send` 是 fire-and-forget，AI 增量走的是全局 EventBus 广播。
   连上 WS 后**必须再发一帧订阅 EventBus** 才收得到流（前端也是这么做的）。详见 §5.3。
3. 新增代码集中在**一个插件目录 + 设置页 3 处登记**，其余机制全部白拿。

---

## 2. 现状机制（本方案的依托，均为既有代码）

| 机制 | 位置 | 本方案如何依托 |
|---|---|---|
| 分形路由唯一入口 | `root.route(ctx)`；入口参数是上下文键值对（`symbio_core/keys.rs`） | 服务只做"请求 → `SimpleRequest`"翻译，业务零改动 |
| 插件注册清单 | `plugins/home/plugin.rs:48` `ensure_defaults()` 的 `defaults` 数组 | 加一项 `"http_api"` 即纳入插件体系（存量 config.yaml 由 `obj.entry().or_insert_with` 自动补齐） |
| 插件构造 | `Composite::build` 按 config 逐项 `create_object` | HTTP 插件的 `enabled` 直接来自持久化配置 |
| 插件配置协议 | `config/get` / `config/set`（`symbio_core/paths.rs`） | 实现这两个路由即可被设置页读写 |
| 设置页分区清单 | `plugins/setting/plugin.rs:153` `SETTING_SECTIONS` | 加一项即出现在设置页 |
| 设置页表单定义 | `plugins/setting/plugin.rs:215` `config_definition()`（`binding: "config"`，`load_path/save_path` = `<prefix>/config get\|set`） | 加一个 `http_api_detail_definition()`，**前端 `DetailForm.vue` 自动渲染** |
| 全局注册表先例 | `HomedirRegistry`（`symbio_core/homedir.rs:173`） | 仿同样式新增 `RootRegistry`，解决"插件拿不到 root" |
| 连接管理 | `RouteConnectionManager`（纯 tokio，不依赖 tauri） | 直接复用 |
| 事件总线 | `EventBus` + `event_bus/subscribe` | 直接复用（AI 流的信源） |

---

## 3. 插件化落点

### 3.1 新增文件

```
symbio/src/plugins/http_api/
├── mod.rs
├── plugin.rs        // HttpApiPlugin：Plugin trait + config get/set + 生命周期
├── config.rs        // HttpApiConfig { enabled, bind, port, token, cors_origins, readonly }
├── server.rs        // axum Router：/api/v1/invoke、/api/v1/ws、/api/v1/events、/api/v1/health
├── ws.rs            // WebSocket 会话：帧协议 ↔ root.route ↔ RouteConnectionManager
└── auth.rs          // Bearer token 校验中间件
```

`symbio_core/ids.rs` 增加 `pub const PLUGIN_HTTP_API: &str = "http_api";`

### 3.2 改动点（**仅 4 处，且都在既有登记位**）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `plugins/mod.rs` | 加 `mod http_api;` |
| 2 | `plugins/home/plugin.rs:48` | `defaults` 数组加 `"http_api"` |
| 3 | `plugins/setting/plugin.rs:153` | `SETTING_SECTIONS` 加 `("http_api", "开放接口")` |
| 4 | `plugins/setting/plugin.rs::detail_definition` | 加 `"http_api" => Some(http_api_detail_definition())` |

设置页表单字段（`config binding`，前缀 `http_api`）：

| 字段 | widget | 默认 |
|---|---|---|
| `enabled` | toggle | `false`（默认关闭） |
| `bind` | text | `127.0.0.1` |
| `port` | number | `9231` |
| `token` | password | 首次启用时自动生成 |
| `cors_origins` | text | 空（仅允许 localhost） |
| `readonly` | toggle | `false` |

### 3.3 依赖（feature gate，与"插件保持可选"一致）

```toml
# symbio/Cargo.toml
[features]
default = []
http-api = ["dep:axum", "dep:tower-http"]

[dependencies]
axum       = { version = "0.8", features = ["ws"], optional = true }
tower-http = { version = "0.6", features = ["cors", "trace"], optional = true }
```

- **全纯 Rust，无 C 编译**（hyper / tokio 系）。
- `ws` feature 由 axum 自带，无需额外 tungstenite 依赖。
- 未开 feature 时插件编译为 stub（`route` 直接返回 `NotFound`），不引入任何 HTTP 依赖。

---

## 4. 生命周期（关键设计）

### 4.1 拿到 root：新增 `RootRegistry`

**问题**：子插件的 `ctx.parent()` 只能拿到直接父级（worker `Composite`），
拿不到 root（`HomePlugin`）。而第三方需要访问 `entities/providers`、`home/*` 等 root 级路由。
`reload` 时 root 还会整个重建，捕获的 `Arc<dyn Plugin>` 会变成"旧 root"。

**方案**：仿 `HomedirRegistry` 在 `symbio_core` 加全局弱引用登记表：

```rust
// symbio_core/root.rs
pub struct RootRegistry;
impl RootRegistry {
    pub fn set(root: &Arc<dyn Plugin>)      // HomePlugin 构造/reload 完成时登记
    pub fn get() -> Option<Arc<dyn Plugin>> // 每次请求时取当前 root
}
```

- 用 `Weak<dyn Plugin>`，不产生循环引用。
- `home/reload` 重建后自动指向新 root，HTTP 服务无需重启即可路由到新插件树。
- 与 `HomedirRegistry` 同一风格，不引入新概念。

### 4.2 启动 / 重启 / 停止

| 时机 | 行为 |
|---|---|
| `build(ctx)` | 读 config；若 `enabled` 且 `tokio::runtime::Handle::try_current()` 可用 → `spawn` server（保存 `JoinHandle` + `CancellationToken`）。无 runtime（如纯同步单测）则标记"待启动"，首次 `route` 时补启动 |
| `config/set` | 落盘（走 parent 的 `save_config`，与 `SettingPlugin::CONFIG_SET` 同款）→ **自行 stop 旧 server + start 新 server**（端口/开关变更必须重启监听，不能像普通配置那样只改内存） |
| `home/reload` | 插件实例被重建（worker composite 清空重建），旧实例 `Drop` → cancel token → 端口释放；新实例按新配置启动 |
| `Drop` | `CancellationToken::cancel()` + `abort` task |
| 客户端断开 | axum 感知 → `remove_connection`（**不** abort 后端任务，与 `tauri://destroyed` 行为一致：AI 继续跑完并持久化） |

> ⚠️ 注意：改配置**不会**自动触发 `home/reload`（`reload` 只在切 homedir 时用）。
> 所以"开关/端口热生效"必须由插件自己在 `config/set` 里完成，这是插件自治，符合机制。

---

## 5. 接口设计

统一前缀 `/api/v1`。所有请求需 `Authorization: Bearer <token>`（loopback 且未设 token 时可放行，便于本机脚本）。

### 5.1 一次性调用（HTTP）

```http
POST /api/v1/invoke
{ "path": "session/get_messages",
  "payload": { "session_id": "s_xxx" },
  "session_id": "s_xxx", "agent_id": "...", "workdir": "...", "trace_id": "..." }
```
→ `200 { "ok": true, "type": "data", "data": {...} }`
→ 若后端返回 `Session` 通道：消费到 EOF 后返回最后一帧（语义对齐前端 `callPlugin`）
→ 若返回 `Native`：`501 { "code": "native_payload" }`（进程内对象不可序列化，**不静默返回 null**）

### 5.2 WebSocket（主通道）

```
WS /api/v1/ws?token=<token>
```

帧协议**直接复用 `PluginMessage` 语义**，与前端 `plugin.ts` 同构：

**客户端 → 服务端**

```jsonc
{ "type": "invoke",     "id": "req-1", "path": "session/chat/send",
  "payload": {...}, "metadata": { "session_id": "s_xxx" } }
{ "type": "send",       "connection_id": "route_conn_7", "frame": { "Data": {...} } }
{ "type": "close",      "connection_id": "route_conn_7" }
```

**服务端 → 客户端**

```jsonc
{ "type": "response",   "id": "req-1", "payload": { "type": "Data", "data": {...} } }
{ "type": "connection", "id": "req-1", "connection_id": "route_conn_7" }
{ "type": "frame",      "connection_id": "route_conn_7", "frame": { "Data": {...} } }
{ "type": "eof",        "connection_id": "route_conn_7" }
{ "type": "error",      "id": "req-1", "message": "..." }
```

服务端内部：`invoke` → `RootRegistry::get()` → `root.route(ctx)`，
返回值落到 `RouteConnectionManager`（复用现有注册/超时清理），
`Session` 的 rx 由 pump 任务转成 `frame` 消息推回该 WebSocket。

### 5.3 ⚠️ AI 流式输出：WebSocket 之后还必须订阅 EventBus

**这是本设计最容易被误解的一点，务必注意。**

- `session/chat/send`（`plugins/session/orchestrator.rs::handle_chat_send_oneoff`）是
  **fire-and-forget**：它 `spawn` 后台任务后**立即返回** `{"status":"accepted"}`，
  **不会**返回一个可以读流的 Session 通道。
- AI 的增量事件由 `run_chat_loop_task` 通过 `EventBus::publish` **广播**给所有订阅者。
- 前端正是靠启动时建的那条 `event_bus/subscribe` 长连接才看到流式输出。

**因此第三方在连上 WebSocket 后，必须先订阅：**

```jsonc
→ { "type": "invoke", "id": "sub-1", "path": "event_bus/subscribe", "payload": { "kinds": ["session"] } }
← { "type": "connection", "id": "sub-1", "connection_id": "route_conn_1" }
← { "type": "frame", "connection_id": "route_conn_1",
     "frame": { "Data": { "type": "bus_event",
                          "data": { "kind": "session", "session_id": "s_xxx", "data": {...} } } } }
```

- 事件按 `session_id` 关联，第三方需自行过滤（前端同款逻辑）。
- 断线重连可调用 `event_bus/pending/snapshot` 补拉最近 64 帧（现有机制，`EventBus::drain_pending`）。
- 副作用是好事：订阅后能收到**所有**会话的事件（包括前端自己触发的），天然具备"观察者"能力。

> 换个说法：**WebSocket 把"通道"问题解决了，但"信源"仍然是 EventBus。**
> 设计里必须显式包含订阅这一步，否则实现的人会以为连上 WS 发个 chat/send 就能收到流——收不到。

### 5.4 SSE（可选，P2）

`GET /api/v1/events`（`text/event-stream`）= `event_bus/subscribe` 的只读 SSE 包装。
保留价值：`curl -N` 即可调试，无需 WS 客户端。**不影响 WebSocket 为主通道。**

### 5.5 运维端点

```
GET /api/v1/health  → { "ok": true, "version": "...", "subscribers": 2, "connections": 3 }
GET /api/v1/manifest → 路由清单（见 §7）
```

---

## 6. 安全

- **默认关闭**（`enabled: false`），设置页显式开启。
- 默认绑定 `127.0.0.1`；改为 `0.0.0.0` 需在描述里警示，且要求 token 非空。
- **token**：首次启用时由插件生成（UUID v4）并随 `save_config` 持久化；设置页以 `password` 控件展示。
  重置 = 清空该字段后保存 → `config/set` 检测空值自动生成新的 → 前端重新 load 显示。
- CORS：默认仅 localhost，可配白名单。
- **只读模式**：仅放行 `entities/list|get|status|detail`、`session/get_messages`、`config/get`、
  `manifest`、`health`；其余 403。
- 危险操作分级（P3）：`local/shell` 类、`work/set_workspace`、`entities/delete`、`config/set`
  归为 `danger`，readonly 与受限 token 拒绝。
- 审计：复用现有 `trace_id` 机制，所有 invoke 记入 tracing。
- **HTTPS**：rustls 默认后端 aws-lc-rs 依赖 `aws-lc-sys`（C），与"无 C 编译"铁律冲突。
  策略：默认纯 HTTP + 回环绑定；远程/HTTPS 交由外部反代（Caddy / ssh tunnel / cloudflared）。

---

## 7. 开放问题：路由清单（manifest）

路由是运行时分形分发，各插件内部 `match path`，**无静态注册表** → 无法自动生成 OpenAPI。

| 做法 | 评价 |
|---|---|
| v1 先不做，第三方照抄前端调用 | 可接受 |
| **给 `Plugin` trait 加可选 `fn manifest(&self) -> Vec<RouteMeta>`（默认空），Composite 递归聚合** | ✅ 最符合项目"机制化"风格：一处定义、自动聚合、前端与新传输层同时受益 |
| 手写 OpenAPI 分片 | 易与代码漂移 |

建议 v1 采用第一种，代码里预留第二种的接口。

---

## 8. 落地步骤

| 阶段 | 内容 | 产出 |
|---|---|---|
| **P0 抽象** | 从 `commands.rs::route_v2` 抽出 `build_context(metadata, payload)` + `dispatch_payload()` 到 `symbio_core/dispatch.rs`；Tauri 命令改为调用它 | 行为零变更，为插件铺路（单独 PR，`cargo test --lib` 全绿） |
| **P1 骨架** | 新增 `RootRegistry`；建 `http_api` 插件（config + 空的 Plugin impl）；登记 4 处改动点 | 设置页出现"开放接口"分区，能读写配置，**还不监听** |
| **P2 通流** | `axum` + `POST /api/v1/invoke` + `WS /api/v1/ws` + token 鉴权；`config/set` 重启逻辑 | **验收：wscat 连上 → 订阅 bus → curl 发消息 → 收到流式事件** |
| **P3 完备** | `health` / `manifest`；SSE 调试通道；只读模式；CORS；审计日志；官方 TS/Python SDK 示例 | 文档与 SDK |
| **P4 增强** | 危险操作分级；manifest 自省机制；Webhook 回调 | — |

### 验收标准（P2）

```bash
# 1) 设置页 → 开放接口 → 开启，得到 token 与端口
# 2) 健康检查
curl -H "Authorization: Bearer $T" http://127.0.0.1:9231/api/v1/health

# 3) 连 WebSocket：先订阅 EventBus
wscat -c "ws://127.0.0.1:9231/api/v1/ws?token=$T"
> {"type":"invoke","id":"1","path":"event_bus/subscribe","payload":{"kinds":["session"]}}

# 4) 发消息（HTTP，fire-and-forget）
curl -H "Authorization: Bearer $T" -X POST http://127.0.0.1:9231/api/v1/invoke \
     -d '{"path":"session/chat/send","session_id":"s_xxx","payload":{"message":"你好"}}'
# → {"ok":true,"type":"data","data":{"status":"accepted",...}}

# 5) 回到 wscat 窗口：应持续收到 kind=session 的增量事件
```

---

## 9. 风险登记

| 风险 | 等级 | 缓解 |
|---|---|---|
| 开放接口 = 第三方可执行 shell / 写文件 | **高** | 默认关闭 + 回环绑定 + token + 只读模式 + 危险分级 |
| 插件拿不到 root / reload 后 root 失效 | 中 | `RootRegistry`（Weak 登记），reload 后自动指向新 root |
| `build` 时无 tokio runtime（单测/CLI） | 中 | `Handle::try_current()` + 延迟到首次 route 补启动 |
| 端口占用 / 重启端口未释放 | 中 | `CancellationToken` + `abort` + 启动时 `SO_REUSEADDR` 探测并回错误到设置页 |
| HTTPS 与"无 C 编译"冲突 | 中 | 默认回环 HTTP，TLS 交外部反代 |
| 第三方以为连上 WS 就有 AI 流 | 中 | §5.3 显式设计 + SDK 内置"连接即自动订阅"默认行为 |
| `chat/send` 无 `is_working` 守卫，多客户端并发互踩 | 低 | 文档声明需自行串行；P4 会话租约 |
| 路由无注册表 → 文档漂移 | 低 | P4 manifest 机制 |
