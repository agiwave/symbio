# HTTP(S) 开放接口插件设计

> 目标：在 App 运行期间对外提供一套纯 HTTP/WebSocket 接口，让第三方应用像前端一样访问和控制整个应用。
> 接口服务本身作为**一个普通插件**纳入现有插件体系，配置/开启/关闭**复用现有设置页机制**，前端零改动。

- 现状：已实现（`gateway` 插件）。现行端点清单见 [reference/ROUTES.md](../reference/ROUTES.md)，配置键见 [reference/CONFIGURATION.md](../reference/CONFIGURATION.md)，调用链见 [architecture/DATA_FLOW.md](../architecture/DATA_FLOW.md)，模块机制见 [gateway 插件 README](../../symbio/src/plugins/gateway/README.md)
- 关联：[architecture/PROTOCOLS.md](../architecture/PROTOCOLS.md)（`PluginMessageWire` / `PluginPayloadWire` / `PluginFrame` 线上类型）

---

## 1. 结论（TL;DR）

1. **做成插件完全可行，且是更优解**——设置页的配置 UI 是现成机制，前端真的不用动。
2. **WebSocket 是对的**，它确实是本场景的最优传输。
   但有个必须写进设计的细节：**WebSocket 解决的是"通道"，不解决"信源"**——
   `session/chat/send` 是 fire-and-forget，AI 增量走的是全局 EventBus 广播。
   连上 WS 后**必须再发一帧订阅 EventBus** 才收得到流（前端也是这么做的）。详见 §5.3。
3. 全部代码集中在**一个插件目录**（`symbio/src/plugins/gateway/`），其余机制全部复用。

---

## 2. 依托的现行机制

| 机制 | 位置 | 本接口如何依托 |
|---|---|---|
| 分形路由唯一入口 | `root.route(ctx)`；入口参数是上下文键值对（`symbio_core/keys.rs`） | 服务只做"请求 → `SimpleRequest`"翻译，业务零改动 |
| 插件注册清单 | `plugins/home/plugin.rs` `ensure_defaults()` 的默认插件数组 | `gateway` 在数组内，随 `worker` 一并实例化（存量 config.yaml 由 `obj.entry().or_insert_with` 自动补齐） |
| 插件构造 | `Composite::build` 按 config 逐项 `create_object` | 网关的 `enabled` 直接来自持久化配置 |
| 插件配置协议 | `config/get` / `config/set`（`symbio_core/paths.rs`） | 实现这两个路由即可被设置页读写 |
| 设置页分区清单 | `plugins/setting/plugin.rs` `SETTING_SECTIONS` | `("gateway", "开放接口")` 是其中一个分区 |
| 设置页表单定义 | `plugins/setting/plugin.rs` `config_definition()`（`binding: "config"`，`load_path/save_path` = `<prefix>/config get\|set`） | 网关表单由后端下发定义，**前端 `DetailForm.vue` 自动渲染** |
| 宿主级上下文注册表先例 | `HomedirRegistry`（`symbio_core/homedir.rs`） | 全局弱引用登记表的同款风格（见 §4.1 的 `parent` 转发） |
| 连接管理 | `RouteConnectionManager`（tauri 宿主层，纯 tokio） | 宿主层用它管前端流式连接；网关在 WS 循环内自持连接生命周期 |
| 事件总线 | `EventBus` + `event_bus/subscribe` | 直接复用（AI 流的信源，见 §5.3） |

---

## 3. 插件落点

```
symbio/src/plugins/gateway/
├── mod.rs        // 插件职责与"只搬运行李"的设计说明
├── plugin.rs     // GatewayPlugin：Plugin trait + config get/set + 生命周期
├── config.rs     // GatewayConfig（扁平 inbound_* 键）+ 只读白名单
└── server.rs     // 手写 HTTP/1.1 + WebSocket：/api/v1/invoke、/api/v1/ws、/api/v1/health
```

- 插件 id：`PLUGIN_GATEWAY = "gateway"`（`symbio_core/ids.rs`）。
- 配置键、默认值与只读白名单：见 [reference/CONFIGURATION.md](../reference/CONFIGURATION.md)「Gateway 插件」；设置页表单由 `setting` 下发的 `DetailDefinition` 渲染。
- **零新依赖、纯 Rust**（手写 HTTP/1.1 + WS 帧解析，不引 axum / tungstenite），与"无 C 编译"铁律一致。

---

## 4. 生命周期（关键设计）

### 4.1 拿到转发目标：`parent.route()`

**问题**：子插件的 `ctx.parent()` 只能拿到直接父级（worker `Composite`），
拿不到 root（`HomePlugin`）。而第三方需要访问 `home/*`、`work/*` 等 root 级路由。
`reload` 时 root 还会整个重建，捕获的 `Arc<dyn Plugin>` 会变成"旧 root"。

**做法**：网关不取 root，而是持有构造期登记的 `Weak` 父级（worker `Composite`），
每次请求 `parent.route(ctx)` —— `Composite` 的路径合并语义与 `root.route` 等价，
home 级路径（`home/*`、`work/*`、`save_config`）由 `Composite`
未命中时的兜底上行转发覆盖。父级以 `Weak` 保存，不产生循环引用；
`home/reload` 重建插件树时网关实例随之重建，转发目标恒为当前树。

### 4.2 启动 / 重启 / 停止

| 时机 | 行为 |
|---|---|
| `build(ctx)` | 读 config；`inbound_enabled` 且 `inbound_protocol = http` → `spawn` server（保存 `JoinHandle` + `CancellationToken`）；`native` 或关闭则不监听 |
| `config/set` | 落盘（走 parent 的 `save_config`，与 `SettingPlugin` 同款）→ **自行 stop 旧 server + start 新 server**（端口/开关变更必须重启监听，不能像普通配置那样只改内存） |
| `home/reload` | 插件实例被重建（worker composite 清空重建），旧实例 `Drop` → cancel token → 端口释放；新实例按新配置启动 |
| `Drop` | `CancellationToken::cancel()` + `abort` task |
| 客户端断开 | 连接读写半关闭 → 结束该会话的 pump 任务（**不** abort 后端任务，与 `tauri://destroyed` 行为一致：AI 继续跑完并持久化） |

> ⚠️ 注意：改配置**不会**自动触发 `home/reload`（`reload` 只在切 homedir 时用）。
> 所以"开关/端口热生效"必须由插件自己在 `config/set` 里完成，这是插件自治，符合机制。

---

## 5. 接口设计

统一前缀 `/api/v1`。设了 `inbound_token` 时 HTTP 走 `Authorization: Bearer <token>`、
WS 走 `?token=<token>`；令牌为空表示不校验（仅允许回环绑定，见 §6）。

### 5.1 一次性调用（HTTP）

请求体就是前端的 `PluginMessageWire`，没有任何新形状：

```http
POST /api/v1/invoke
{ "metadata": { "path": "session/get_messages", "session_id": "s_xxx", "trace_id": "..." },
  "payload":  { "session_id": "s_xxx" } }
```

→ `200` + `PluginPayloadWire`：`{ "type": "Data", "data": {...} }`
→ 后端返回 `Session` 通道时：服务端消费到 EOF（30s 超时兜底）后回**最后一帧**（语义对齐前端 `callPlugin` 的一次性调用）
→ 后端返回 `Native` 时：`400 {"error":"该路径返回进程内原生对象，不支持跨传输调用"}`（**不静默返回 null**）

### 5.2 WebSocket（主通道）

```
WS /api/v1/ws?token=<token>
```

**一条 WS = 一个会话**，帧载荷与前端完全同构，不引入第二套帧协议：

1. 客户端**首帧**发 `PluginMessageWire`（与 `route_v2` 请求体一字不差）；
2. 后端返回 `Session` 通道 → 该连接此后**双向转发 `PluginFrame`**
   （`{"Data": …}` / `{"Error": [message, details]}`），会话结束即关闭；
3. 后端返回一次性数据 → 回一帧后关闭。

并发请求由客户端开多条连接承担，因此不需要 `connection_id` / 请求 id 一类的复用机制。
这也是"只搬运行李，不改行李内容"的落点：任何能走 `route_v2` 的路径都能原样走 WS。

服务端内部：首帧 → `build_ctx`（`metadata` 逐键注入上下文）→ `parent.route(ctx)` →
按 `PluginPayload` 四态分派（见 §4.1）。

### 5.3 ⚠️ AI 流式输出：WebSocket 之后还必须订阅 EventBus

**这是本设计最容易被误解的一点，务必注意。**

- `session/chat/send`（`plugins/session/orchestrator.rs::handle_chat_send_oneoff`）是
  **fire-and-forget**：它 `spawn` 后台任务后**立即返回** `{"status":"accepted"}`，
  **不会**返回一个可以读流的 Session 通道。
- AI 的增量事件由 `run_chat_loop_task` 通过 `EventBus::publish` **广播**给所有订阅者。
- 前端正是靠启动时建的那条 `event_bus/subscribe` 长连接才看到流式输出。

**因此第三方在连上 WebSocket 后，必须先订阅**（专门用一条连接收事件）：

```jsonc
→ { "metadata": { "path": "event_bus/subscribe" }, "payload": { "kinds": ["session"] } }
← { "Data": { "type": "bus_event",
              "data": { "kind": "system", "session_id": null,
                        "data": { "event": "connected", "connection_id": "…" } } } }
← { "Data": { "type": "bus_event",
              "data": { "kind": "session", "session_id": "s_xxx", "data": {…} } } }
```

- 订阅是**全量**的：`SubscribeRequest.kinds` 当前不参与过滤，事件按 `session_id`
  关联，消费方需自行过滤（前端同款逻辑）。
- 断线重连可按会话调用 `event_bus/pending/snapshot`（载荷 `{ "session_id": … }`）
  补拉最近 64 帧（`EventBus::drain_pending`，取走即清空）。
- 副作用是好事：订阅后能收到**所有**会话的事件（包括前端自己触发的），天然具备"观察者"能力。

> 换个说法：**WebSocket 把"通道"问题解决了，但"信源"仍然是 EventBus。**
> 设计里必须显式包含订阅这一步，否则实现的人会以为连上 WS 发个 chat/send 就能收到流——收不到。

### 5.4 运维端点

```
GET /api/v1/health  → { "ok": true }
```

### 5.5 预留通道（未实现）

- **SSE**：`GET /api/v1/events`（`text/event-stream`）= `event_bus/subscribe` 的只读包装。
  价值在 `curl -N` 即可调试，无需 WS 客户端；**WebSocket 仍是主通道**。
- **manifest**：`GET /api/v1/manifest` → 路由清单（见 §7）。
- 运行状态（订阅者数、是否在监听）经 `gateway/status` 路由取，不在 health 里。

---

## 6. 安全

- **默认关闭**：`inbound_enabled = false` 且 `inbound_protocol = native`（不监听任何端口），
  设置页「开放接口」显式开启。
- **绑定地址**：默认 `127.0.0.1`。绑定非回环地址却未设 `inbound_token` →
  `server::start` 直接返回错误（安全护栏），设置页可见。
- **token**：`inbound_token` 由用户在设置页填写（`password` 控件）并随 `save_config` 持久化；
  HTTP 走 `Authorization: Bearer <token>`，WS 走 `?token=`；置空即不校验（仅回环允许）。
- CORS：响应头固定 `Access-Control-Allow-Origin: *`（`OPTIONS` 预检直接放行），
  跨域隔离依赖 token 与绑定地址，不靠 Origin 白名单。
- **只读模式**（`inbound_readonly`）：仅放行 `is_readonly_allowed` 白名单
  （清单见 [CONFIGURATION.md](../reference/CONFIGURATION.md)），其余 403/拒绝；
  `gateway/config/*` 一律拒绝（`config/get` 含 `inbound_token`）。
- 危险操作分级（`local/shell` 类、`work/set_workspace`、`vdfs/write|delete`、`config/set`
  归为 `danger`）：**预留**，现行只有只读白名单一层。
- 审计：`trace_id` 随 `metadata` 透传进上下文，invoke 记入 tracing。
- **HTTPS**：不做。rustls 默认后端 aws-lc-rs 依赖 `aws-lc-sys`（C），与"无 C 编译"铁律冲突。
  策略：默认纯 HTTP + 回环绑定；远程/HTTPS 交由外部反代（Caddy / ssh tunnel / cloudflared）。

---

## 7. 开放问题：路由清单（manifest）

路由是运行时分形分发，各插件内部 `match path`，**无静态注册表** → 无法自动生成 OpenAPI。

| 做法 | 评价 |
|---|---|
| **不做，第三方照前端调用方式拼路径**（现行） | 可接受：路径清单在 [ROUTES.md](../reference/ROUTES.md) 人工维护 |
| 给 `Plugin` trait 加可选 `fn manifest(&self) -> Vec<RouteMeta>`（默认空），Composite 递归聚合 | ✅ 最符合项目"机制化"风格：一处定义、自动聚合、前端与新传输层同时受益 |
| 手写 OpenAPI 分片 | 易与代码漂移 |

现行采用第一种；第二种是 manifest 的方向。

---

## 8. 联调示例

```bash
# 1) 设置页 → 开放接口 → 协议选 http、填 token 与端口并保存
# 2) 健康检查
curl -H "Authorization: Bearer $T" http://127.0.0.1:9231/api/v1/health

# 3) 连 WebSocket：一条连接专门订阅 EventBus
wscat -c "ws://127.0.0.1:9231/api/v1/ws?token=$T"
> {"metadata":{"path":"event_bus/subscribe"},"payload":{"kinds":["session"]}}

# 4) 另开一条连接发消息（fire-and-forget）
curl -H "Authorization: Bearer $T" -X POST http://127.0.0.1:9231/api/v1/invoke \
     -d '{"metadata":{"path":"session/chat/send","session_id":"s_xxx"},"payload":{"message":"你好"}}'
# → {"type":"Data","data":{"status":"accepted",...}}

# 5) 回到订阅那条 wscat 窗口：应持续收到 kind=session 的 bus_event 帧
```

---

## 9. 风险登记

| 风险 | 等级 | 缓解 |
|---|---|---|
| 开放接口 = 第三方可执行 shell / 写文件 | **高** | 默认关闭 + 回环绑定 + token + 只读模式（危险分级为预留层） |
| 插件拿不到转发目标 / reload 后失效 | 中 | 构造期登记 `Weak` 父级（`parent.route`），reload 时网关实例随树重建 |
| 端口占用 / 重启端口未释放 | 中 | `CancellationToken` + `abort` + 启动失败信息回写到设置页 |
| HTTPS 与"无 C 编译"冲突 | 中 | 默认回环 HTTP，TLS 交外部反代 |
| 第三方以为连上 WS 就有 AI 流 | 中 | §5.3 显式设计 + SDK 内置"连接即自动订阅"默认行为 |
| `chat/send` 无 `is_working` 守卫，多客户端并发互踩 | 低 | 文档声明需自行串行；预留方向：会话租约 |
| 路由无注册表 → 文档漂移 | 低 | 预留方向：manifest 机制（§7） |
