---
kind: design
id: DE-013-HTTP开放接口
title: HTTP 开放接口
status: active
created: 2026-09-06
impl: done
---

# HTTP 开放接口

> 目标：在 App 运行期间对外提供一套纯 HTTP/WebSocket 接口，让第三方应用像前端一样访问和控制整个应用。
> 接口服务本身作为**一个普通插件**纳入现有插件体系，配置/开启/关闭**复用现有设置页机制**，前端零改动。

- 现状：已实现（`gateway` 插件）。现行端点清单见 [路由参考](../004-ref/003-ROUTES.md "RE-003-ROUTES")，配置键见 [配置参考](../004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION")，调用链见 [数据流与调用链](003-数据流与调用链.md "DE-003-数据流与调用链")，模块机制见 [gateway 插件 README](../../crates/symbio-plugin-gateway/README.md "crates/symbio-plugin-gateway/README")
- 关联：[协议规范](005-协议规范.md "DE-005-协议规范")（`PluginMessageWire` / `PluginPayloadWire` / `PluginFrame` 线上类型）

---

## 1. 结论（TL;DR）

1. **做成插件完全可行，且是更优解**——设置页的配置 UI 是现成机制，前端真的不用动。
2. **WebSocket 是对的**，它确实是本场景的最优传输。
 但有个必须写进设计的细节：**WebSocket 解决的是"通道"，不解决"信源"**——
 往收件箱写消息（`vdfs/write <会话地址>/inbox`）是 fire-and-forget，AI 增量走的是
 VDFS 变更（`kind = "vdfs"`）。连上 WS 后**必须订阅总线 + 对会话地址登记 `vdfs/watch`**
 （两步缺一不可）才收得到流（前端也是这么做的）。详见 §5.3。
3. 全部代码集中在**一个插件目录**（`crates/symbio-plugin-gateway/src/`），其余机制全部复用。

---

## 2. 依托的现行机制

| 机制 | 位置 | 本接口如何依托 |
|---|---|---|
| 分形路由唯一入口 | `root.route(ctx)`；入口参数是上下文键值对（`symbio_core/keys/`） | 服务只做"请求 → `PluginSimpleRequest`"翻译，业务零改动 |
| 必需插件清单 | `plugins/home/plugin.rs` 的 `SYSTEM_PLUGINS`，经 ctx 键 `REQUIRED_PLUGINS` 随构造传入 | `gateway` 在清单内，容器据此补目录 / 配置文件并实例化（容器**不内置**任何清单） |
| 插件构造 | `Composite::build` **扫描自己的目录（系统根）**，逐目录 `creator_create_object`，并把插件自身目录经 `PLUGIN_DIR` 告知它 | 网关的开关就是它自己 `PLUGIN.yml` 里的 `inbound_enabled` |
| 插件配置 | **没有第二条配置协议**：配置 = 插件目录里的 `PLUGIN.yml`（`<根>/gateway/PLUGIN.yml`），读写走 `vdfs/read` / `vdfs/write` | 网关只要把自己的 `PluginConfigFile` 声明出去即可被设置页与 LLM 同时读写 |
| 设置页清单 | `ConfigurableVisitor` 收集通道（`symbio_core/capability/configurable.rs`）：插件在 `traverse` 里 `capability_announce_configurable` 一次 | 网关的「开放接口」自动出现在设置页，**前端零改动** |
| 设置页表单 | 由配置的**拥有者**产出 `DetailDefinition`（作为节点 `schema` 下发） | 网关表单由后端下发定义，前端表单渲染器自动渲染 |
| 宿主级上下文注册表先例 | `HomedirRegistry`（`symbio_core/homedir.rs`） | 全局弱引用登记表的同款风格（见 §4.1 的 `parent` 转发） |
| 连接管理 | `RouteConnectionManager`（tauri 宿主层，纯 tokio） | 宿主层用它管前端流式连接；网关在 WS 循环内自持连接生命周期 |
| 事件总线 | `EventBus` + `event_bus/subscribe` + `vdfs/watch` | 直接复用（AI 流的信源 = VDFS 变更，见 §5.3） |

> 依托点**不是** `config/get` / `config/set` 协议、`SETTING_SECTIONS` 分区表或
> `load_path` / `save_path`——这三样都不存在（配置已回到插件目录）。本设计的结论是
> **做成插件、复用既有机制、前端零改动**，依托点是 VDFS 的配置文件 + 可配置收集通道，见 [VDFS 前端页面规范](010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §3 地址模型 / §2 分层与落点。

---

## 3. 插件落点

```
crates/symbio-plugin-gateway/src/
├── mod.rs // 插件职责与"只搬运行李"的设计说明
├── plugin.rs // GatewayPlugin：Plugin trait + 生命周期 + 配置文档声明
├── config.rs // GatewayConfig（扁平 inbound_* 键）+ 只读白名单
└── server.rs // 手写 HTTP/1.1 + WebSocket：/api/v1/invoke、/api/v1/ws、/api/v1/events、/api/v1/health
```

- 插件 id：`PLUGIN_ID_GATEWAY = "gateway"`（装配方 `crates/crates/crates/assembly.rs`）。
- 配置键、默认值与只读白名单：见 [配置参考](../004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION")「Gateway 插件」；设置页表单由**配置的拥有者**下发的 `DetailDefinition` 渲染。
- **零新依赖、纯 Rust**（手写 HTTP/1.1 + WS 帧解析，不引 axum / tungstenite），与"无 C 编译"铁律一致。

---

## 4. 生命周期（关键设计）

### 4.1 拿到转发目标：`parent.route`

**问题**：子插件的 `ctx.parent` 只能拿到直接父级（worker `Composite`），
拿不到 root（`HomePlugin`）。第三方经 HTTP 能到达哪些地址，就由此划定。
`reload` 时 root 还会整个重建，捕获的 `Arc<dyn Plugin>` 会变成"旧 root"。

**做法**：网关不取 root，而是持有构造期登记的 `Weak` 父级（worker `Composite`），
每次请求 `parent.route(ctx)` —— `Composite` 按首段在**已挂载子插件**里分发，
未命中直接 `NotFound`（源码 `plugins/composite/composite.rs` 的 `route`）。
父级以 `Weak` 保存，不产生循环引用；`home/reload` 重建插件树时网关实例随之重建，
转发目标恒为当前树。

**由此划定的可达面**：入站请求到得了 worker 子树里的每条插件路由，**到不了 home
自己的四条臂**（`home/get_homedir` · `home/reload` · `home/set_workspace` ·
`home/get_workspace`）——home 是 composite 的**父**，而 `Composite::route` 不向上
转发（实测回 400，判据 e2e `t69` ⑦）。那四条臂的生产驱动面是前端的**进程内**
`callPlugin`（`tauri/src/services/home.ts` ⇒ `root.route`），不经 HTTP。
若将来要把边界抬到 root，先重审只读白名单（[RE-001](../004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION") 的「白名单与可达面必须一致」）再改这里。

### 4.2 启动 / 重启 / 停止

| 时机 | 行为 |
|---|---|
| `build(ctx)` | 读 config；`inbound_enabled` 且 `inbound_protocol = http` → `spawn` server（保存 `JoinHandle` + `CancellationToken`）；`native` 或关闭则不监听 |
| 配置写入 | `vdfs/write` `<根>/gateway/PLUGIN.yml` → `PluginConfigFile::apply` 落盘 → 本插件在自己的 `write` 返回后 **stop 旧 server + start 新 server**（端口/开关变更必须重启监听，不能像普通配置那样只改内存） |
| `home/reload` | 插件实例被重建（worker composite 清空重建），旧实例 `Drop` → cancel token → 端口释放；新实例按新配置启动 |
| `Drop` | `CancellationToken::cancel` + `abort` task |
| 客户端断开 | 连接读写半关闭 → 结束该会话的 pump 任务（**不** abort 后端任务，与 `tauri://destroyed` 行为一致：AI 继续跑完并持久化） |

> ⚠️ 注意：改配置**不会**自动触发 `home/reload`（`reload` 只在切 homedir 时用）。
> 所以"开关/端口热生效"必须由插件自己在写配置之后完成，这是插件自治，符合机制
> ——机制不引入回调抽象，`PluginConfigFile::apply` 只管「校验 → 落内存 → 落自己的文件 →
> 广播」。

---

## 5. 接口设计

统一前缀 `/api/v1`。设了 `inbound_token` 时 HTTP、WS 与 SSE 都走
`Authorization: Bearer <token>`（WS 浏览器客户端走 `Sec-WebSocket-Protocol: bearer.<token>`，
见 §5.2；浏览器 `EventSource` 不能自定义头，故 SSE 面向非浏览器客户端，见 §5.5）。
令牌为空表示不校验（仅允许回环绑定，见 §6）。**令牌不进查询串**——
查询串会进访问日志、代理记录与 `Referer`。

### 5.1 一次性调用（HTTP）

请求体就是前端的 `PluginMessageWire`，没有任何新形状：

```http
POST /api/v1/invoke
{ "metadata": { "path": "vdfs/read", "session_id": "s_xxx", "trace_id": "..." },
 "payload": { "path": "<根>/session/s_xxx" } }
```

→ `200` + `PluginPayloadWire`：`{ "type": "Data", "data": {...} }`
→ 后端返回 `Session` 通道时：服务端消费到 EOF（30s 超时兜底）后回**最后一帧**（语义对齐前端 `callPlugin` 的一次性调用）
→ 后端返回 `Native` 时：`400 {"error":"该路径返回进程内原生对象，不支持跨传输调用"}`（**不静默返回 null**）

#### 5.1.1 重放防护：`Idempotency-Key`

`POST /api/v1/invoke` 可带请求头 `Idempotency-Key`（1–200 字节，可见 ASCII）。缺省时行为
与不带本头逐字相同——**这是能力，不是强制**。

**语义裁决：拒绝重放，不缓存旧结果**（锁式，不是缓存式）。三条理由：

1. **两种传输必须同一条规则**：WS 是双向长连接，没有「重放同一条连接」这回事；本入口的
 HTTP 响应还可能是流式会话折叠后的末帧。缓存式会在 HTTP 上成立、在 WS 上不成立，
 同一份「幂等」于是要写两遍、且必然漂移。
2. **缓存式会把客户端 bug 变成静默的错误答案**：同一把键换了请求体时，返回旧结果看起来
 像成功。拒绝式让「键用错了」当场可见（`409`）。
3. **与 `store` 的幂等语义同形**：只认「同一身份已存在就拒绝」，不返回旧结果。

时限与边界：

- 首次带键 ⇒ **先登记再分派**（并发重放在登记处就被挡住，不靠执行完再比）；
- 分派**失败**（含请求体非法）⇒ **释放键**——失败请求的重试是合法的；
- 分派成功 ⇒ 键保留 **10 分钟**（上限 4096 个，超出先清到期、再逐出最旧）；
- 键表是**进程内**的：重启后不存在。跨重启的 exactly-once 不由本传输承诺——那需要持久化
 键表，等于新增存储面，不在本设计内。

### 5.2 WebSocket（主通道）

```
WS /api/v1/ws
Authorization: Bearer <token>
```

**一条 WS = 一个会话**，帧载荷与前端完全同构，不引入第二套帧协议：

1. 客户端**首帧**发 `PluginMessageWire`（与 `route_v2` 请求体一字不差）；
2. 后端返回 `Session` 通道 → 该连接此后**双向转发 `PluginFrame`**
 （`{"Data": …}` / `{"Error": [message, details]}`），会话结束即关闭；
3. 后端返回一次性数据 → 回一帧后关闭。

并发请求由客户端开多条连接承担，因此不需要 `connection_id` / 请求 id 一类的复用机制。
这也是"只搬运行李，不改行李内容"的落点：任何能走 `route_v2` 的路径都能原样走 WS。

**令牌的两种携带方式**（WS 握手不能自定义任意头，但**能**带 `Authorization` 与子协议）：

| 客户端 | 怎么带 | 说明 |
|---|---|---|
| 非浏览器（CLI / SDK / curl / wscat） | `Authorization: Bearer <token>` | 与 HTTP 同一规则 |
| 浏览器 `WebSocket`（不能设自定义头） | `Sec-WebSocket-Protocol: symbio.v1, bearer.<token>` | 服务端选 `symbio.v1` 回包，`bearer.*` 只用于鉴权、**不被选为子协议** |

- **查询串令牌不再接受**（`?token=`）：它会进访问日志、代理记录与 `Referer`。
- 子协议鉴权时，客户端必须至少再提供一个非 `bearer.` 的子协议（文档示例给 `symbio.v1`）；
 没有可回选的子协议时，服务端不回 `Sec-WebSocket-Protocol` 头，浏览器会判握手失败——
 这是有意的：**令牌不能靠「被选中」回显**。
- 两者都给时以 `Authorization` 为准；都不给且已配置令牌 ⇒ `401`。

服务端内部：首帧 → `build_ctx`（`metadata` 逐键注入上下文）→ `parent.route(ctx)` →
按 `PluginPayload` 四态分派（见 §4.1）。

### 5.3 ⚠️ AI 流式输出：WebSocket 之后还必须**再开一条** `vdfs/watch`

**这是本设计最容易被误解的一点，务必注意。**

- 往收件箱写消息（`vdfs/write <根>/session/<id>/inbox`）是 **fire-and-forget**：
 后台任务受理后**立即返回**，**不会**返回一个可以读流的 Session 通道。
 （该写入面上不存在同义的 `session/chat/send` 路由与 `handle_chat_send_oneoff`，
 见 [D-066](012-会话输入与地址.md "DE-012-会话输入与地址")。）
- AI 的增量**不在**那条写入连接上。会话实时面——消息转写**与会话运行态**——
 是 **VDFS 变更**（`kind = "vdfs"`），落在会话子树 `<根>/session/<id>` 上
 （`<根>` 是对外展示根，运行期取回，不写字面量——见 `docs/004-ref/003-ROUTES.md`）。

**因此第三方必须再建一条 WS 连接，两步缺一不可**：

1. `event_bus/subscribe` —— 开**通道**（载荷空，按 `kind` 在客户端过滤）；
2. `vdfs/watch` —— 开**闸**：后端只向**登记过路径**的订阅者投递变更。

```jsonc
// 第二条连接：订阅总线 + 登记会话子树（发消息那条用完即走，不是同一条）
→ { "metadata": { "path": "event_bus/subscribe" }, "payload": {} }
→ { "metadata": { "path": "vdfs/watch" }, "payload": { "path": "<根>/session/<id>" } }
// 此后该连接逐帧下发 PluginFrame::Data：
← { "Data": { "type": "bus_event", "data": { "kind": "vdfs", "session_id": null,
 "data": { "path": "<根>/session/<id>/message/m1", "data": { "id": "m1", "content": "第一" } } } } }
← { "Data": { "type": "bus_event", "data": { "kind": "vdfs", "session_id": null,
 "data": { "path": "<根>/session/<id>/message/m1", "data": { "id": "m1", "delta": "第二" } } } } }
← { "Data": { "type": "bus_event", "data": { "kind": "vdfs", "session_id": null,
 "data": { "path": "<根>/session/<id>", "data": { "…": "全量节点视图" } } } } } // 运行态：随载荷落定
```

- **信封没有操作枚举**（S27）：形状 `{path, data?}`。**`path` 恒为被变更节点自身的
 地址**——会话是容器，其下是若干并列的集合（消息 / 子会话 / 记忆 / 工作目录……），
 集合项的形状统一为 `<sid>/<集合段>/<项 id>`，消息帧的落点就是 `<sid>/message/<mid>`
 这个节点，**身份在地址末段**（与 `data.id` 同一事实）。`data` 就是那条 `ChatNode`：
 `delta` ⇒ **尾部追加**（零回读）、`content` ⇒ 整条替换、`status = removed` ⇒ 就地
 移除；运行态帧落在会话节点 `<sid>` 上，`data` 是全量节点视图，就地落定。
- **无载荷变更** ⇒ 回读 / 重拉（`vdfs/read` 拿到的是**已含增量**的正文）；
 资源删除回读 `stat` 得 `NotFound` 即删除。
- **不要按到达顺序拼**：每条变更都带 `path`，节点**自带位置**（`ChatNode.seq`）。
 两个并行工具的变更会**混着到**、后生成的**先到**，按 `path` 分派、按 `seq` 排序即可。
- **丢了怎么办**：`event_bus` 通道满时**不摘除订阅**，会补送一条 resync 指令
 （`data.data.type == "resync"`）——收到就按自己的作用域**整份重读**（幂等，安全）。

> 换个说法：**WebSocket 把"通道"问题解决了，但"信源"是 `vdfs/watch` 上的会话子树。**
> 设计里必须显式包含「另开一条连接 + `vdfs/watch` 登记」这两步，否则实现的人会以为
> 连上 WS 发个 `chat/send` 就能收到流——收不到。

> **为什么实时面是 `vdfs/watch`，不是另立一条流式协议**：订阅写进代码很容易，
> 但代价是把**数据的属性**当成**传输的属性**——顺序是**节点属性**（`ChatNode.seq`），
> 「追加」不是新的事件取值而是 `updated` 上的 `delta` 字段（[D-025](005-协议规范.md "DE-005-协议规范")）。
> 一旦另立 `session/stream` 这类订阅面，同一份顺序语义就要在传输层再实现一次，
> 而第二个信源（运行态）还得再立一条。实时面因此只有 `vdfs/watch` 一条。

### 5.4 运维端点

```
GET /api/v1/health → { "ok": true }
```

### 5.5 SSE 实时面（只读）

```
GET /api/v1/events
Authorization: Bearer <token>
→ 200 text/event-stream
data: {"type":"bus_event","data":{"kind":"vdfs","session_id":null,"data":{"path":"<根>/session/<id>/…"}}}

data: {"type":"bus_event","data":{"kind":"system","session_id":null,"data":{"event":"connected","connection_id":"…"}}}
```

= `event_bus/subscribe` 的**只读包装**，不新增信源、不新增帧形状：

- 每条 `data:` 承载**总线信封原样**（`{type:"bus_event", data:{kind, session_id, data}}`），
  `kind` 由消费方自行分派；WS 客户端看到的是同一份东西，只是外层多包一层
  `{"Data": …}`。不写 `event:` 具名事件——那会让 `EventSource.onmessage` 收不到。
- **要收到会话流仍需 `vdfs/watch`**（见 §5.3）：总线只是广播口，本端点不登记 watch、
  也不按 `kind` 过滤。
- 鉴权与 HTTP 同一规则（`Authorization`，**不接受查询串令牌**）。浏览器 `EventSource`
  不能自定义头，因此浏览器仍走 WS 子协议携带令牌；SSE 面向 `curl -N` / SDK 调试。
- **只读模式拒绝**（`403`）：它是长连接广播，一次订阅覆盖作用域内全部会话的在途内容
  （含尚未落库的增量），与「逐个地址去读」的只读口径不同（同一条取舍见 §6 与
  [配置参考](../004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION")）。
- 空闲期每 15s 发一行注释 `: ping` 保活；连接断开（读到 FIN 或写失败）即取消订阅令牌，
  总线侧的反注册任务随之摘除订阅者——**不留幽灵订阅**。
- 连接上限与 WS 同一配额（`inbound_max_connections` 计一条存活连接）。

### 5.6 预留通道（未实现）

- **manifest**：`GET /api/v1/manifest` → 路由清单（见 §7）。
- 运行状态（订阅者数、是否在监听）经 `gateway/status` 路由取，不在 health 里。

---

## 6. 安全

- **默认关闭**：`inbound_enabled = false` 且 `inbound_protocol = native`（不监听任何端口），
 设置页「开放接口」显式开启。
- **绑定地址**：默认 `127.0.0.1`。绑定非回环地址却未设 `inbound_token` →
 `server::start` 直接返回错误（安全护栏），设置页可。
- **token**：`inbound_token` 由用户在设置页填写（`password` 控件）并随配置文件落盘
 （`<根>/gateway/PLUGIN.yml`）；HTTP 与 WS 都走 `Authorization: Bearer <token>`
 （浏览器 WS 走 `Sec-WebSocket-Protocol: bearer.<token>`，见 §5.2）；**不接受查询串令牌**；
 置空即不校验（仅回环允许）。
- **连接上限**（`inbound_max_connections`，缺省 64）：同时存活的连接数上限，HTTP 一次调用
 占一个、WS 与 SSE 各占一条从握手活到关闭的连接。到顶时新连接留在内核 accept 队列里等（**背压优先于
 报错**：拒绝本身也需要一条连接），已有连接不受影响。配置变更经写配置后的重启监听生效。
- **重放防护**：`/api/v1/invoke` 的 `Idempotency-Key`（语义与边界见 §5.1.1）。
- CORS：响应头固定 `Access-Control-Allow-Origin: *`（`OPTIONS` 预检直接放行），
 跨域隔离依赖 token 与绑定地址，不靠 Origin 白名单。
- **只读模式**（`inbound_readonly`）：仅放行 `is_readonly_allowed` 白名单
 （清单见 [配置参考](../004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION")），其余 403/拒绝；
 `gateway/*` 一律拒绝，`GET /api/v1/events` 也拒绝（长连接广播），且 `vdfs/read`
 只要落在任何插件的 `PLUGIN.yml` 上就拒绝（配置含 `inbound_token` 等凭据）。
- 危险操作分级（`local/cmd`（Windows）/ `local/sh` 类、`vdfs/write|delete` 归为
 `danger`）：**预留**，现行只有只读白名单一层。home 的臂不在任何分级里——它经入站
 根本走不到（§4.1）。
- 审计：`trace_id` 随 `metadata` 透传进上下文，invoke 记入 tracing。
- **HTTPS**：不做。策略：默认纯 HTTP + 回环绑定；远程/HTTPS 交由外部反代（Caddy / ssh tunnel / cloudflared）。
 依赖不是阻碍——HTTP 出口的 TLS 走平台原生栈，不引入 C 编译（[D-013](../../crates/symbio-plugin-model/README.md "crates/symbio-plugin-model/README")）；若要让网关
 自带 HTTPS，需另行设计证书配置面（签发方式 / 证书存放 / 与 `inbound_token` 的关系 / 绑定非回环时的
 取舍）——那是产品决策，不是依赖问题。

---

## 7. 开放问题：路由清单（manifest）

路由是运行时分形分发，各插件内部 `match path`，**无静态注册表** → 无法自动生成 OpenAPI。

| 做法 | 评价 |
|---|---|
| **不做，第三方照前端调用方式拼路径**（现行） | 可接受：路径清单在 [RE-003-ROUTES](../004-ref/003-ROUTES.md "RE-003-ROUTES") 人工维护 |
| 给 `Plugin` trait 加可选 `fn manifest(&self) -> Vec<RouteMeta>`（默认空），Composite 递归聚合 | ✅ 最符合项目"机制化"风格：一处定义、自动聚合、前端与新传输层同时受益 |
| 手写 OpenAPI 分片 | 易与代码漂移 |

现行采用第一种；第二种是 manifest 的方向。

---

## 8. 联调示例

```bash
# 1) 设置页 → 开放接口 → 协议选 http、填 token 与端口并保存
# 2) 健康检查
curl -H "Authorization: Bearer $T" http://127.0.0.1:9231/api/v1/health

# 3) 连 WebSocket：令牌走 Authorization 头（浏览器改走子协议，见 §5.2），
# 订阅总线，再对会话地址登记 watch（两步缺一不可，见 §5.3）
wscat -c "ws://127.0.0.1:9231/api/v1/ws" -H "Authorization: Bearer $T"
> {"metadata":{"path":"event_bus/subscribe"},"payload":{}}
> {"metadata":{"path":"vdfs/watch"},"payload":{"path":"<根>/session/s_xxx"}}

# 4) 另开一条连接发消息（fire-and-forget；收件箱地址面，`session_id` 由地址末段给）。
# 需要重试语义时带 Idempotency-Key：同一把键的第二次投递回 409（见 §5.1.1）。
curl -H "Authorization: Bearer $T" -H "Idempotency-Key: $(uuidgen)" \
 -X POST http://127.0.0.1:9231/api/v1/invoke \
 -d '{"metadata":{"path":"vdfs/write","session_id":"s_xxx"},"payload":{"path":"<根>/session/s_xxx/inbox","text":"你好"}}'
# → {"type":"Data","data":{...}}（受理即回；正文走第 3 步那条连接的变更帧）

# 5) 回到订阅那条 wscat 窗口：应持续收到 kind=vdfs 的 bus_event 变更帧
```

---

## 9. 风险登记

| 风险 | 等级 | 缓解 |
|---|---|---|
| 开放接口 = 第三方可执行 shell / 写文件 | **高** | 默认关闭 + 回环绑定 + token + 只读模式（危险分级为预留层） |
| 插件拿不到转发目标 / reload 后失效 | 中 | 构造期登记 `Weak` 父级（`parent.route`），reload 时网关实例随树重建 |
| 端口占用 / 重启端口未释放 | 中 | `CancellationToken` + `abort` + 启动失败信息回写到设置页 |
| ~~HTTPS 与"无 C 编译"冲突~~ **已消解** | 低 | 依赖不再是阻碍（[D-013](../../crates/symbio-plugin-model/README.md "crates/symbio-plugin-model/README")：TLS 已走平台原生栈）；现状仍默认回环 HTTP，TLS 交外部反代 |
| 第三方以为连上 WS 就有 AI 流 | 中 | §5.3 显式设计 + SDK 内置"连接即自动订阅"默认行为 |
| `chat/send` 无 `is_working` 守卫，多客户端并发互踩 | 低 | 文档声明需自行串行；预留方向：会话租约 |
| 路由无注册表 → 文档漂移 | 低 | 预留方向：manifest 机制（§7） |
| **请求重放 / 网络层重试导致同一次写入被处理两次** | **高** | `Idempotency-Key`（§5.1.1，拒绝重放）；边界：键表在进程内，跨重启不保证——需要时先补持久化裁决 |
| **并发连接洪峰打穿进程（无上限）** | **高** | `inbound_max_connections` + `Semaphore` 背压（§6）；上限是运行时参数，不是新增机制键 |
| **令牌进日志（查询串 / 代理记录 / Referer）** | **高** | 令牌只走 `Authorization` 头或 WS 子协议（§5.2），查询串令牌不再接受；日志不含头 |
