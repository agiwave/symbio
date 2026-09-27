# 宿主接缝定向调查：RouteBridge 该不该抽（2026-09-27）

> 本报告是 [`architecture-optimization-review-2026-09-27.md`](architecture-optimization-review-2026-09-27.md)
> §5 的**前置设计调查**。复核报告在 §5 提议抽一个跨宿主的 `RouteBridge`；
> 本报告用实测数字核实该提议是否成立。
>
> **结论是不抽**——详见 §5。此处保留完整依据，供将来重新评估。

---

## 一、三方接线对照（实测）

| 维度 | Tauri 壳 | CLI | 网关 |
|---|---|---|---|
| 位置 | `tauri/src-tauri/src/`（449 行 / 3 文件） | `cli/src/client.rs`（686 行） | `symbio/src/plugins/gateway/server.rs`（794 行） |
| 持有根 | `AppState.root: Arc<dyn Plugin>` | `SymbioClient.root: Arc<dyn Plugin>` | `parent: Weak<dyn Plugin>` |
| 请求入口 | 3 个 IPC command：`route_v2` / `route_v2_send` / `route_v2_close` | 无独立入口，`route<T>()` 一个泛型方法 | HTTP 手写路由 + `WS /api/v1/ws` |
| 信封 | 有（`PluginMessageWire` 进出） | **无**（强类型直传） | 有（`PluginMessageWire` → `PluginPayloadWire`） |
| 流回推 | `app.emit("route/{conn_id}")` | 解成 `cli::Frame` → stdout | `ws_send_text` + `ws_read_frame` |
| 连接身份 | `RouteConnectionManager`（HashMap + `AtomicU64` + 60s 清理） | **无连接概念**（全局 event_bus 订阅） | **无连接 id**（栈帧内的 `handle_ws`） |

**关键事实**：三种「连接身份」模型根本不同——Tauri 有完整生命周期管理，CLI 完全没有，
网关用栈帧局部变量。这不是同一件事的三种写法。

---

## 二、逐字/近逐字相同的代码（唯一的可合并候选）

### 候选 A — `wire → 扩展桶上下文`（**真·近逐字重复**，已核实）

- `tauri/src-tauri/src/commands.rs:53-88`
- `symbio/src/plugins/gateway/server.rs:502-524`

两处语义**完全一致**：`extensions.insert(PLUGIN_PAYLOAD_KEY, Arc::new(payload))`
+ 遍历 `metadata`，`is_str` 存 `Arc<String>`，否则存 `Arc<Value>`，最后构造
`PluginSimpleRequest { envs, extensions }`。

**差异点仅 3 处，均非语义**：
1. Tauri 版 `payload` 按值移动（`request.payload`），gateway 版 `msg.payload.clone()`
   ——因为 gateway 后续还要复用 `msg`；
2. Tauri 有多一段 trace_id / origin 提取与日志（不在共性内）；
3. Tauri 版 `unwrap_or(&Map::new())` vs gateway 版 `if let Some(obj)`
   ——同一语义两种写法。

**Tauri 侧代码里已有一条注释**：「桶名与 gateway 的 `build_ctx` 同一份契约」——
说明这个共享契约是**已知的**，只是没有被机制化。

**可合并**：提为 `symbio_core` 的一个 `fn ctx_from_wire(msg: &PluginMessageWire)`，
两处各减 ~25 行。

### 候选 B — `PluginPayload` 三态分类

`gateway/server.rs:415-451` 已自行抽出 `classify_payload` + `PayloadDelivery`（43 行），
其文档明确写着「从前它写在两个传输入口里（HTTP 与 WS 各一份）」——

**但 Tauri 侧没有接上**：`commands.rs:96-149` 手写了同一个三态 `match`。

**可合并的只有 `Data → serialize()` 与 `Empty → Null` 两臂**。
`Session` 臂**必须分叉**：Tauri 要注册连接并保留通道，gateway HTTP 要折叠为最后一帧。

### 候选 C/D/E — 判定**不可合并**

| 候选 | 判定 | 依据 |
|---|---|---|
| C 帧的 JSON 编解码 | **已收口** | 类型住在 `symbio_core/plugin/transport.rs`；Tauri 侧的编解码根本不在 Rust（在 `plugin.ts` 的 `JSON.stringify`）。Rust 内不存在第二份 |
| D 错误 → 响应帧映射 | **不能合并** | 四种形状：Tauri `Err(String)` IPC 错误 / gateway HTTP `{"error":…}` + 400 / gateway WS `{"Error":[…]}` / CLI 转 stderr。无语义一致的一对 |
| E 连接注册 / 生命周期 | **Tauri 单边** | 只有 Tauri 有连接表；另外两个宿主没有对应物，不是三方共性 |

---

## 三、传输相关、不可合并（清单 + 证据）

### Tauri 专属
- `AppHandle` / `Emitter::emit` / `tauri::State`；
- 事件名 `route/{conn_id}` + `/eof` —— IPC 事件通道名，**无跨传输对应物**；
- `register_fixed` 握手竞态修复 —— 只因为 IPC 必须「先 listen 再 invoke」才需要。

### CLI 专属
- stdin/stdout 分流、`is_terminal()`、`ExitCode`、REPL 循环；
- **刻意不装 tracing subscriber**（让插件日志退到 stderr 保 stdout 纯净）——与协议无关；
- CLI **不消费 `PluginFrame` 本身**：它解成自己的 `cli::Frame`（`Change`/`Resync`），
  语义层比 frame 高一层。

### 网关专属
- HTTP 状态码 / CORS / `Content-Length` 手动读体 / `MAX_*_BYTES`；
- WS 握手（`Sec-WebSocket-Accept` + 内联 SHA1，约 60 行）与帧掩码编解码；
- 鉴权双形态：`Bearer`（HTTP 头）+ `?token=`（WS 查询参数，因握手不能自定义头）；
- 只读模式闸门。

### 背压/流控：三种不同机制（关键证据）

| 宿主 | 通道满时 | 代码 |
|---|---|---|
| CLI（event_bus） | **静默丢帧**，保留订阅，补发 resync 标记，靠回读 `vdfs/stat` 自愈 | `event_bus/mod.rs:146-169` |
| Tauri | 无背压 —— `app.emit` fire-and-forget，前端丢帧后**后端收不到任何信号** | `commands.rs:125-129` 仅 `warn!` |
| 网关 WS | 天然背压 = TCP 窗口 + `.await`，**无丢帧、无 resync 语义** | `server.rs:621` |

**这是三个已文档化的、各有理由的机制**，不是「同一件事的三种实现」。

---

## 四、已有底座（可复用评估）

| 类型 | 位置 | 评估 |
|---|---|---|
| `PluginMessageWire` / `PluginPayloadWire` / `PluginFrame` | `symbio_core/plugin/transport.rs` | **已是共享线上格式**，三方共用 |
| `PluginChannel` | 同上 | 会话通道**唯一原语** |
| `PLUGIN_PAYLOAD_KEY` | `symbio_core/plugin/mod.rs:175` | 上下文桶名唯一常量 |
| `classify_payload` / `PayloadDelivery` | `gateway/server.rs:415-451` | **半成品**：注释自称从两个入口提上来的，但只服务 gateway 内部 |
| `build_ctx` | `gateway/server.rs:502` | 与 Tauri `commands.rs` 语义重复，但住在 gateway 里 |
| `route_connection.rs`（167 行） | `tauri/src-tauri/src/` | **不是半成品共享原语**——它绑定 Tauri IPC 的 emit/连接语义，另外两个宿主用不到。它待在 Tauri 是**正确归属** |

---

## 五、结论：**不抽 `RouteBridge`**

### 判据

按本仓核心判据「**有没有真逻辑差异**」裁定：**三者根本不同构**。
README 的「同构」只成立于**线上类型 + 路由入口**这一层，
**不成立于宿主接缝这一层**。

### 抽得出来的一半（真·无逻辑差异）

1. `wire → Arc<PluginSimpleRequest>`（候选 A）——唯一逐字可合并的块；
2. `PluginPayload` 的 `Data` / `Empty` 序列化臂（候选 B 的一半）。

### 抽不出来的另一半（真差异，必须留在宿主）

1. **会话通道的消费动作**：Tauri 注册连接并 emit / gateway HTTP 折叠为最后一帧 /
   gateway WS 双向 select / CLI 根本不走这条（它订阅 event_bus）
   ——**四个消费动作，四种语义**；
2. **背压策略**：三种不同且各有理由的机制（见 §三）；
3. **连接身份**：只有 Tauri 有；
4. **错误形状**：四种；
5. **全部 transport I/O**。

### 为什么造 `RouteBridge` 是错的

强行造统一 `RouteBridge`，在 `Session` 分支上会**立即逼出一个带三个 `match host`
的上帝函数**——把「三种机制各自的理由（丢帧 / resync / 阻塞）」压成一处 switch。
**收益约 40 行，代价是消灭三份已文档化的差异说明。**

这正是本仓反复出现的同一条判据（§3 的「九个工具不全收」、§7 的「上层解析不合并」）：
**判据是「有没有真逻辑差异」，不是「行数够不够少」。**

### 值得做的（低风险去重，约 -35 行）

| 做什么 | 依据 | 预估 |
|---|---|---|
| 把候选 A 提到 `symbio_core`（如 `transport::ctx_from_wire`） | 两处 32+23 行近逐字重复，Tauri 侧已有注释承认同一契约 | -35 行 |
| 把 `classify_payload` 从 gateway 提到 `symbio_core` | gateway 文档已自述「从两个入口提上来」，Tauri 侧仍手写同一 match | -10 行 |

**明确不做**：`RouteBridge` 不持有连接表、不启动转发泵、不定义事件名。
那三件事分别是 `RouteConnectionManager`（Tauri 专属）、`handle_ws`（gateway 专属）、
`SymbioClient.events`（CLI 专属）。

**`route_connection.rs` 保持 Tauri 专属**——它是 IPC 语义的正确归属，不是半成品。
