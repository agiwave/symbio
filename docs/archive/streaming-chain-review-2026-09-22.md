# 流式消息链路评审（2026-09-22）

> **文档类型：评审记录（点时刻）** — 回答「流模式下前后端链路是什么样、是否合理、是否过长、协议上还有多少空间」。
> 结论基于对当前 HEAD 的代码通读（`symbio/` + `tauri/` + `tauri/src-tauri/`），不含推测。
> 现行行为以 `docs/architecture/DATA_FLOW.md`、`docs/architecture/PROTOCOLS.md`、
> `symbio/src/plugins/session/docs/node-state-streaming.md` 为准；本文只做评估与建议。
>
> ⚠️ **后记（S26 / ADR-025，2026-09-23）：本文的架构基线已被取消。**
> 本文把「实时面两条通道」当作问题、并建议把它们**合并**（§4.1 是当时判定的最高价值
> 改进点）。合并（批次 E）做过，**S26 又整体反向**：实时面迁回 **VDFS 变更**
> （`updated` + `delta`），`session/stream` 与 `symbio_core/transcript_stream.rs` 退役。
> **本文 §4.1 的整条推理因此作废**——它假设「跨通道顺序假设」是个真问题，而顺序是
> **节点属性**（`ChatMessage.seq`）不是投递属性，那个假设不存在。
> 本文对**信封重量 / serde 遍历 / 深拷贝**的实测与优化（O1–O6）仍然有效，
> 其中已落地的部分（`Arc<Value>` 扇出、一次序列化）**不随本次反向而回退**。

---

## 0. 结论摘要

| 问题 | 结论 |
|---|---|
| 链路**是否合理** | **基本合理**。执行期已收敛为 `EventSink`/`AbortSignal` 两个原语，帧语义收成「一条消息」，转写只有一个写入点，这些是高质量设计。 |
| 链路**是否过长** | **环节数偏多（17 步），但冗余不多**。真正可压的是「信封层数」与「每帧的重复拷贝/序列化」，不是环节本身。 |
| **架构**优化空间 | **有，且集中在一处**：实时面拆成两条通道后，两条的投递保证强度不对等（见 §4.1）。这是当前最高价值的结构性改进点。 |
| **协议**优化空间 | **有，中等**。信封过重（≈110 字节承载 1 字节）、后端每帧 2 次 serde 遍历 + 逐订阅者深拷贝、三种传输（Tauri/HTTP/WS）响应容器不一致。 |
| 是否发现**缺陷** | 发现 **2 个可复现的实现缺陷**（§4.1、§4.3）与 **3 处死代码**（§4.3、§4.5、§4.6）。 |

### 0.1 落地状态（2026-09-22 更新）

本轮按 §7 的顺序实际落地了 A–D 四批，外加两处评审中未列出的缺陷；
**E 已在后续批次落地**（见下），**F 的处置与理由**见 §7.1。逐项见下表。

| 批次 | 内容 | 状态 | 关键落点 |
|---|---|---|---|
| **A** | `event_bus` 满通道不再摘除订阅 + 限流告警 + 补送 resync 标记 | ✅ 已落地 | `symbio_core/event_bus.rs`；前端 `eventBus.ts` / `sessionNodeSync.ts` / `useVdfs.ts` 接 resync 指令 |
| **B** | `PluginChannel.cancel_token` 真正接线；删 `actualId` 死分支 | ✅ 已落地 | `route_connection.rs` / `commands.rs` / `plugin.ts` |
| **C** | 前端转写帧批处理（合帧窗口 48ms） | ✅ 已落地 | `services/transcriptStream.ts` / `stores/sessions.ts::applyTranscriptMessages` |
| **D** | 后端出帧**一次**序列化 + `Arc` 扇出（零深拷贝） | ✅ 已落地 | `symbio_core/transport.rs`（`PluginFrame::Data(Arc<Value>)`）/ `transcript_stream.rs` / `event_bus.rs` |
| — | **§4.2 的短期缓解**：前端补 `reconcileTranscript`（跨通道顺序假设的自愈网） | ✅ 已落地 | `services/transcriptStream.ts::reconcileTranscript` + `stores/sessions.ts::scheduleReconcileTranscript` |
| — | **新发现（P1，评审时未列）**：Telegram 侧转写帧解包层级错，导致每条消息都回「无响应」 | ✅ 已修复 | `plugins/telegram/plugin.rs` |
| — | **§4.9 的结构性收尾**：三份手写解包副本收敛到 `symbio_core` 公共入口；生产者侧 `build_envelope` 收成唯一构建入口 | ✅ 已落地 | `transcript_stream::{event_of,is_resync,EVENT_TYPE,RESYNC_TYPE}` / `vdfs_provider::vdfs_change_of` / `event_bus::build_envelope`（改 `pub`） |
| — | §4.7 `kinds` 死字段 | ✅ 已删除（未实现，见下） | `symbio_core/event_bus.rs::SubscribeRequest` |
| — | §4.8 `PROTOCOLS.md` 命名约定过时 | ✅ 已修正 | `docs/architecture/PROTOCOLS.md` |
| — | §4.5 `Native` 载荷两种传输处置不一致 | ✅ 已统一（改为两侧一致报错） | `tauri/src-tauri/src/commands.rs` |
| **E** | 会话运行态并入转写流、删第二条实时通道 | ✅ 已落地（含 VDFS 侧不再携带节点快照） | `transcript_stream::{SessionStateEvent,publish_session_state}` / `transcript.rs::emit_session_state` / `transcriptStream.ts`（`advanceSeq` 两种帧共用）/ `sessions.ts::applySessionState`；规范见 `node-state-streaming.md` §11 |
| **F** | 传输容器统一（响应恒为 `PluginMessageWire`） | ⏸ 未做，理由见 §7.1（**维持原判定**：那条不对称是有意的；两条子项已二次复核，均不成立） | **F 域内唯一真缺陷已修**：`docs/design/http-api-transport.md` §5.3 对外接入指南在 E 之后与实现不符（还在教人订阅 VDFS 看流式），已整段改写 |

**验证**：`cargo test --lib` 908 通过 ｜ 前端 683 通过（46 文件）+ `vue-tsc` 干净 ｜
e2e 11/11 通过（**用重新构建的 `cli/target/release` 二进制**，不是门禁里那个陈旧产物）。
（**E 落地后**：915 通过 ｜ 687 通过 ｜ e2e 11/11，T9 另增「两种帧共用一个 `seq` 空间」
与「收尾帧 seq > 全部消息帧 seq」两条断言。）

---

## 1. 全链路：一次增量从 LLM 到像素

### 1.1 步骤表（17 步，标注必要性）

| # | 环节 | 位置 | 必要性 |
|---|---|---|---|
| 1 | 上游 SSE 字节流 | 远端 LLM | 必需 |
| 2 | 按 `\n` 切行 + 逐行 JSON 解析 | `symbio_core/turn.rs:795` `parse_sse_stream` | 必需（SSE 固有） |
| 3 | 协议适配 → `ProtocolEvent` | `plugins/model/protocols/*.rs`（4 种 `SseLineParser`） | 必需（多协议差异的唯一吸收点） |
| 4 | `dispatch_protocol_event` → 构造 `ChatMessage` | `turn.rs:1042` → `emit_delta`/`emit_message`（`turn.rs:65/84`） | 必需 |
| 5 | `EventSink::Direct` → `TranscriptWriter::apply` | `symbio_core/exec.rs:103` → `session/orchestrator/sink.rs:34` | **设计亮点**（进程内零 serde） |
| 6 | `Transcript::apply` 内存图合并 | `session/transcript.rs:239` | 必需（唯一写入点） |
| 7 | `Transcript::emit` 分配 `seq` + 分级日志 | `session/transcript.rs:395` | 必需 |
| 8 | `publish_frame` 序列化 + 扇出 | `symbio_core/transcript_stream.rs:96` | 必需，**实现可优化** |
| 9 | `mpsc(4096).try_send` | `transcript_stream.rs:117` | 必需 |
| 10 | Tauri 泵 `rx.recv()` → `app.emit("route/{conn}")` | `tauri/src-tauri/src/commands.rs:94-104` | 必需（跨进程边界） |
| 11 | WebView IPC → `listen` 回调 → `ProtocolEnforcer.validate` | `tauri/src/services/plugin.ts:390-397` | 必需，**可与其他步骤合并** |
| 12 | `connectPlugin` 回调 → `ProtocolEnforcer.extract` | `plugin.ts:641` | **与 #11 重复遍历同一帧** |
| 13 | `handleStreamFrame` → `applyNodeEvent`（seq 缺口门） | `services/transcriptStream.ts:117/144` | 必需（丢帧可检测的落点） |
| 14 | `sink.message` → `applyTranscriptMessage` | `stores/sessions.ts:287` | 必需 |
| 15 | `updateMessages` 整份浅拷贝 → `commitMessages` | `stores/sessions.ts:192/176` | **实现可优化**（见 §5.2） |
| 16 | `messageTree` 重建树 + 排序 + 签名 | `composables/useChatConnection.ts:189` | 必需，**实现可优化** |
| 17 | Vue diff → 像素 | Vue runtime | 必需 |

**判断**：17 步中 14 步是结构上必需的。真正「过长」的不是步数，而是 **#11/#12 对同一帧做两遍遍历**、**#8/#15/#16 的实现开销**。链路骨架（执行期原语 → 单一写入点 → 单一帧语义）是干净的。

### 1.2 请求方向

`session/chat/send` 的入站同样是一条 4 层通道：

```
前端 buildMetadata（path/session_id/trace_id/workdir/agent_id）
  → PluginMessageWire { metadata, payload }
  → route_v2：metadata 逐键 + payload 全部塞进 extensions(HashMap<String, Arc<dyn Any>>)   [commands.rs:38-59]
  → root.route(ctx) → home → worker(Composite) → session
  → session/plugin.rs:446 → orchestrator/entry.rs:169 handle_chat_send_oneoff
  → tokio::spawn：emit_session_state(Working) → 落库 → collect_capabilities → run_chat_loop_task
```

注意 `commands.rs:38-59` 对每个 metadata 键做一次 `Arc` 分配，payload 整体 `Arc` 一次。控制面高频调用（`vdfs/watch`、`vdfs/stat`）每次都要付这份成本；量不大，但属于「每请求固定税」。

---

## 2. 流式协议本体

### 2.1 三类帧

| 帧 | 形状 | 语义 |
|---|---|---|
| 数据帧 | `PluginFrame::Data({ type:"transcript_event", data: NodeEvent })` | 帧 = 一条 `ChatMessage` |
| 重同步帧 | `PluginFrame::Data({ type:"transcript_resync", data: null })` | 后端明示「你可能漏了帧」→ 消费端整份重读 |
| 错误帧 | `PluginFrame::Error(msg, { code })` | 带机器可读 `ErrorCode` |

`NodeEvent = { session_id, seq, message }`（`transcript_stream.rs:78`）。

### 2.2 帧语义「全在字段上」（S24）

协议**没有操作枚举**。消费端按字段分派：

| 帧里的字段 | 接收端动作 |
|---|---|
| `delta` | 尾部追加（该内容的首次传输） |
| `content` | 整条替换（幂等） |
| `status = removed` | 就地移除 |
| `status`（其余）/ `error` | 状态迁移 |
| 身份字段 / `meta` / `seq` / `timestamp` | 有则合并 |

`delta` 与 `content` **互斥**；同帧携带即协议违例，后端写入点（`transcript.rs:239`）与前端落地（`sessions.ts:287`）用**同一条判据**报错丢弃。这是一条很好的设计——两端判据同源，协议漂移会当场暴露而不是静默错。

### 2.3 `seq` 的双层语义（易混淆，需注意）

- **外层 `NodeEvent.seq`** = 会话内**帧序号**，单调递增，用于缺口检测；
- **内层 `message.seq`** = **存储排序锚点**，由存储分配。

两者语义不同，故嵌套而非平铺（`transcript_stream.rs:80-84` 有注释说明）。前端 `S.lastSeq: Map<sessionId, number>`（`transcriptStream.ts:87`）只跟外层。**每会话独立 seq 空间**，因 `Transcript` 实例是 per-session。

### 2.4 背压策略（两端互补）

```
后端（transcript_stream.rs:96-167）
  通道满 → 摘除订阅 → 重试投递 resync 标记（20 × 100ms）
          → 送达则重新入表；2s 送不进则永久摘除 + warn
  通道关闭 → 静默摘除

前端（transcriptStream.ts:144-170）
  seq == last+1 → 应用
  seq <= last   → 丢弃（重复帧，防叠字）
  seq >  last+1 → 跳号 = 已知有损 → 整份重读
```

**设计承诺**：「丢帧在机制上不可能发生——要么送达，要么收到 resync，要么摘除留痕」（`transcript_stream.rs:18`）。这条承诺**在转写流上是成立的**。

### 2.5 会话运行态走另一条通道

会话节点的 `status`（`working`/`active`/`failed`）+ `attributes.outcome`/`.error` **不走转写流**，而是：

```
emit_session_state（orchestrator/broadcast.rs:102）
  → notify_session_state（plugin.rs:116）→ session_node（plugin/nodes.rs:145）
  → session_change（nodes.rs:129，VdfsChange::new(id,"updated").with_node(node)，只带 node 不带 content）
  → ChangeSubscriptions::notify（symbio_core/vdfs/host.rs:176）
  → event_bus_sink（plugins/vdfs/host.rs:137）→ EventBus::try_publish(KIND_VDFS, None, data)
  → SUBSCRIBERS[mpsc] → Tauri 泵 → app.emit("route/{conn}") → 前端 eventBus
```

**关键观察**：这条通道**没有 `seq`、没有 resync、没有缺口检测**。它的正确性完全依赖「状态是幂等的全量视图」这一前提。

---

## 3. 做得对的地方（不宜改动）

1. **执行期与传输层分离**（ADR-020）。`EventSink`（出方向）+ `AbortSignal`（入方向）取代 `PluginChannel` 双职责，进程内零 serde、无帧、无轮询。这是链路能保持短的关键。
2. **转写唯一写入点**。`Transcript::apply` 是后端唯一写入点，`applyTranscriptMessage` 是前端唯一落地口，两端判据同源（§2.2）。
3. **单一出口不变量**。会话运行态只有 `emit_session_state` 一个出口；漏调即 UI 永久停旧状态，这条不变量被文档显式锁定（`node-state-streaming.md` §8 #6）。
4. **组合节点终态跟随子树**。`finalize_turn_root` 全流程唯一一处（`chat_loop/io.rs:19`），修掉了「容器已完成、工具仍在跑」的自相矛盾形状。
5. **帧语义收成一条消息**（S24）。删掉 `NodeOp`/`NodeChange`，消费端不必从帧形状推断「该拼接还是该替换」。
6. **日志分级与折行只碰日志**。`seq` 分配与 `publish_frame` 逐帧无例外（不变量 #28），可观测性优化没有污染协议。

---

## 4. 问题清单

### 4.1 【P0】`event_bus` 通道满时静默丢帧 + 静默摘除订阅，且无重连信号

**位置**：`symbio_core/event_bus.rs:82-96`

```rust
for entry in SUBSCRIBERS.iter() {
    let (id, tx) = (entry.key(), entry.value());
    if tx.is_closed() || tx.try_send(frame.clone()).is_err() {
        to_remove.push(id.clone());     // ← Full 与 Closed 同等对待
    }
}
for id in to_remove { SUBSCRIBERS.remove(&id); }   // ← 无日志
```

**问题**：
1. `try_send` 在 `Full` 与 `Closed` 两种情况下都返回 `Err`，代码把两者一并摘除；
2. 摘除**没有任何日志**；
3. 更严重：摘除后**前端不会收到任何信号**。Tauri 连接（`route_v2` 的 Session）仍然活着，泵任务仍在 `rx.recv()` 上等待——没有 EOF、没有 `disconnected`、没有 `error`。前端 `eventBus` 只在收到 `disconnected`/`error` 时才重连（`eventBus.ts:377-388`），因此**前端会以为连接正常，却永久收不到任何 VDFS 变更**（含会话运行态、全部资源变更）。

**为什么这是 P0**：会话运行态走这条通道。一旦静默摘除，`working` 状态永远不会收到终态帧 → 会话卡片与停止按钮永久停在「运行中」，且**没有任何机制会纠正**（`reconcileTranscript` 的触发判据是「会话报不忙」，而「不忙」这一帧本身也丢了）。这正是 `node-state-streaming.md` §8 #20 想防的那类故障。

**讽刺之处**：`transcript_stream.rs:10` 的模块注释明确写着它是「对 `EventBus::try_send` 静默丢帧缺陷的**机制级纠正**」。**只纠正了一条通道，另一条没动**。

**建议**（二选一，成本都很低）：
- **方案 A（对齐转写流）**：`Full` 走 resync 语义 + warn 留痕；只有 `Closed` 才摘除。但 VDFS 变更无 `seq`，resync 需要「重读全部已 watch 路径」的语义，比转写流复杂。
- **方案 B（最小改动，推荐先做）**：`Full` 时**不摘除**（保留订阅），只 warn + 丢这一帧。理由：VDFS 变更是幂等全量视图，丢一帧的代价是「少一次状态更新」，而 `list`/`stat` 快照与下一次变更会自愈（这正是 §4.2 假设 1/2 已接受的模型）；但**摘除订阅**的代价是「永久失联」，两者严重性不对称。同时补上 warn 日志。

### 4.2 【P0】跨通道顺序依赖：`会话不忙 ⇒ 所有节点已终态`

**位置**：`node-state-streaming.md` §6 S20.7 ③ 的触发判据

文档称该判据「不是启发式」：「节点补丁恒先于会话状态下发（正常收尾『清在途 → 复位 `is_working` → `emit_session_state`』）」。

**但两条帧走的是两条不同的通道**：

```
消息终态帧：publish_frame → STREAM_SUBS[mpsc 4096] → 泵任务 A → emit("route/connA")
会话状态帧：try_publish  → SUBSCRIBERS[mpsc 2048] → 泵任务 B → emit("route/connB")
```

两个独立 `mpsc`、两个独立 `tokio::spawn` 泵任务、两个不同事件名。**发送端调用顺序在源头上是确定的，但跨通道到达顺序没有任何机制保证**——它依赖两个泵任务的调度巧合。缓冲区大小不同（4096 vs 2048）、订阅表不同、`notify_change` 走的是 `ChangeSubscriptions` 最长匹配路径，这些都增加了乱序可能。

**后果**：`reconcileTranscript` 是这条顺序假设的**补丁**，而不是「顺序无关」的体现。设计目标（§0「事件可以丢、可以重放、可以乱序，节点状态不会因此错」）在**单条通道内**成立，**跨通道不成立**。

**建议**：
- **短期**：把这条假设**写进文档的不变量**（现在它只以「触发判据不是启发式」的形式隐含在 §6 里），并给 `reconcileTranscript` 补一条兜底：会话 `status` 长时间为 `working` 且转写无变化时也触发一次核对（覆盖 §4.1 的静默摘除）。
- **长期**：见 §6.1 —— 把会话运行态并入转写流，则「顺序无关」在**同一 seq 空间**内成立，这条跨通道假设被整体删除。

> **状态：短期缓解已落地；长期（E 批）未做，理由见 §7.1**。
> 落地的是「兜底」那一半，且实现比原建议更稳：不是「`status` 长时间为 `working`
> 时触发核对」，而是**在 `working → 非 working` 的迁移上挂 300ms 宽限复查**
> （`stores/sessions.ts::scheduleReconcileTranscript`），仍停在非终态才整份回读。
> 这样常态（终态帧只晚到一两个往返）**不产生任何额外读取**，代价只落在真正没收敛的
> 罕见情形上；原建议那种「长时间为 working 就核对」会把正常的长推理也判成异常。

### 4.3 【P1】`PluginChannel.cancel_token` 是死代码

**位置**：`symbio_core/transport.rs:173`；消费点 `plugins/event_bus/plugin.rs:80`、`plugins/session/plugin.rs:231`

`PluginChannel::pair` 创建 `CancellationToken` 并共享给两端，两个插件都 `spawn` 了「等 `cancel_token.cancelled()` 后反注册」的清理任务。

**但全仓没有任何一处对 `PluginChannel` 的 token 调用 `.cancel()`**（已全仓 grep 确认：`.cancel()` 只出现在 gateway/telegram/`exec.rs` 的 `AbortSignal`/`route_connection.rs` 的 `cleanup_token`，均非本 token）。

**后果**：
1. 两个清理任务**永不执行**，是死代码；
2. `session/plugin.rs:227` 的注释「连接断开（cancel_token 触发）时反注册……双保险」与事实不符；
3. 订阅表清空实际只依赖 `publish_frame`/`try_publish` 里的 `tx.is_closed()` 探测——**若之后不再发布任何帧，条目就一直留在表里**。转写流发布频繁尚可自愈；`event_bus` 若无 VDFS 变更则长期泄漏。

**建议**：要么在 `route_v2_close` / 传输泵退出时真正 `cancel()`（推荐——让「连接断开」有主动的、确定的收口），要么删掉 token 与两个清理任务，改为在 `route_connection::remove_connection` 里显式反注册。**不要保留「看起来会清理、实际不会」的代码**——它比没有更危险。

### 4.4 【P1】每帧的重复序列化与深拷贝

**位置**：`transcript_stream.rs:96-128`

```rust
let data = serde_json::to_value(event)               // 第 1 次 serde 遍历（构造 Value）
let frame = PluginFrame::Data(json!({ "type":…, "data": data }))   // 第 2 层 Value
for entry in STREAM_SUBS.iter() {
    tx.try_send(frame.clone())                       // ← 每订阅者一次深拷贝
}
```

再加上 `app.emit(&event_name, &frame)` 内部还会把 `Value` 序列化成 JSON 字符串（**第 2 次完整遍历**）。

所以一个 token 的路径上共有：**2 次 serde 遍历 + N 次深拷贝（N = 订阅者数，典型 1~3：前端 + CLI + 子智能体转播）**，之后才进 WebView。

**建议**：
1. 让 `publish_frame` 接收可直接序列化的结构（如 `NodeEvent` + 一个枚举），避免先 `to_value` 再 `json!` 包装——Tauri 的 `emit` 本身就接受 `Serialize`，直接传 `&Envelope<NodeEvent>` 可省一次遍历；
2. 扇出改为**共享**：先序列化成一次 `Arc<str>`（或 `Arc<Value>`），各订阅者只克隆 `Arc`。若 Tauri 支持 `serde_json::value::RawValue` 作为 payload，可直接把预序列化字节交给 IPC，省掉 emit 内部那次遍历。

> **状态：已落地（第 2 条的建议 1 半 + 建议 2 的前半）**。
>
> - **遍历次数 2 → 1**：`json!` 对**表达式**参数展开为 `to_value(&expr).unwrap()`
>   （serde_json 1.0.151 `macros.rs:278`），所以原来的
>   `json!({ "type": …, "data": data })` 会把已经 `to_value` 过的载荷**再走一遍 serde**。
>   现在改为直接建 `Map` 并把 `Value` **移动**进去：`transcript_stream::envelope`、
>   `event_bus::build_envelope`（后者更进一步——载荷**按所有权**搬入，整条路径零遍历）。
> - **扇出零深拷贝**：`PluginFrame::Data(Value)` → `PluginFrame::Data(Arc<Value>)`，
>   `frame.clone()` 变成引用计数自增。代价是 serde 要开 `rc` feature（已开，
>   `symbio/Cargo.toml` 有说明）。
> - **消费端顺带去掉的深拷贝**：原来每个消费者都用
>   `serde_json::from_value::<T>(x.clone())`，现在统一走 `T::deserialize(x)`（借用
>   `&Value` 反序列化）。这是**每帧**发生的，不比生产端便宜。
> - **未做**：`RawValue` 直通（建议 2 的后半）。它要求 `PluginFrame` 携带预序列化字节，
>   而进程内消费者（`subagent.rs` / `cli/client.rs`）需要**读**帧结构，
>   改完他们反而要反解析一遍——对 Tauri 一条路省一次遍历，对两条进程内路径各加一次。
> - **回归锚点**：`fan_out_shares_one_payload_allocation`（两个订阅者必须
>   `Arc::ptr_eq`）钉住「扇出不复制」，改回逐订阅者克隆立刻变红。

### 4.5 【P2】三种传输的响应容器不一致

| 传输 | 响应形态 | 位置 |
|---|---|---|
| Tauri IPC | `PluginMessageWire { metadata, payload: PluginPayloadWire }` | `commands.rs:77-81` |
| HTTP invoke | **裸** `PluginPayloadWire` | `gateway/server.rs:416-453` |
| WS | **裸** `PluginFrame` | `gateway/server.rs:501-595` |

前端被迫在 `httpInvokeTransport` 里再包回 `{ metadata:{}, payload:wire }` 以求对称（`plugin.ts:495`）——**这是协议不对称的直接证据**。

另外两处语义分叉：
- `PluginPayload::Native`：Tauri 静默返回 `Null`（`commands.rs:111-117`），gateway 明确报错；
- `PluginPayload::Session`：Tauri 建流式连接，gateway 折叠到 EOF 末帧 + 30s 超时——**同一个 `Session` 载荷在两种传输下语义不同**。

**建议**：统一为「响应恒为 `PluginMessageWire`」，把「一次性 vs 流式」的表达收进 `PluginPayloadWire` 而不是靠传输层各自约定。至少让 `Native` 的处置一致。

### 4.6 【P2】死代码：`actualId !== sessionId` 分支不可达

**位置**：`plugin.ts:414-432`

后端 `route_v2` 在 `client_id` 存在时**恒**用 `register_fixed(client_id)` 并返回同一 id（`commands.rs:84-86`），而前端 `nativeTransport` **恒**传 `clientId: sessionId`（`plugin.ts:404-407`）。故 `actualId !== sessionId` 永不成立，那段「无缝切换监听器」约 20 行是死代码。

**风险**：它掩盖了一个真实缺口——若后端将来真改了 id，预注册的监听器在旧事件名上，**切换窗口内的帧会丢**，且 `ProtocolEnforcer` 不会报错。

**建议**：删除该分支，改为在 `route_v2` 的契约里把「conn_id 恒等于 client_id」写成显式约定（并有测试锁定）。

> **状态：已落地**。分支已删，替换为**显式契约断言**——不一致时抛错并说明
> 「预注册监听器会挂在旧事件名上，帧静默丢失」。比原来的「无缝切换监听器」
> 更安全：那条路径从来没被执行过（`route_v2` 在收到 `clientId` 时恒用
> `register_fixed`，前端恒传 `clientId: sessionId`），但它**掩盖了**一个真实缺口。

### 4.7 【P2】`event_bus/subscribe` 的 `kinds` 过滤未实现

**位置**：`plugins/event_bus/plugin.rs:67-68`

```rust
// 预留：基于 _req.kinds 过滤事件（暂未实现，所有事件都推送）
let _ = _req.kinds;
```

前端因此必须在客户端过滤（`eventBus.ts:398-408`），所有 VDFS 变更（含工作目录文件变更等）**全量**跨进程推送。当前 `kind` 闭集只有 2 个值，收益有限；但若未来新增 `kind`，服务端过滤能直接省掉 IPC 体积。

**建议**：要么实现（`DashMap` 的 value 加一个 `kinds: HashSet<String>`），要么把 `SubscribeRequest.kinds` 字段删掉——**保留一个「看起来能用」的字段比没有更容易误导**。

### 4.8 【P2】文档过时表述

`PROTOCOLS.md:274` 称命名遵循「`snake_case`（Rust）↔ `camelCase`（host 转换）」，但 `schemas/` 实际**全程 snake_case**（仅 MCP 外部规格用 camelCase）。建议修正，避免后来者按错误约定写代码。

> **状态：已修正**。核查依据：全仓 `#[serde(rename_all = "camelCase")]` 共 **7 处，
> 全部在 `plugins/mcp/types.rs`**（镜像 MCP 外部规范报文），`symbio_core/schemas/` 里
> **一处都没有**。已把该句改写为「全程 `snake_case`，无 host 侧转换」并注明唯一例外。

### 4.9 【P1】Telegram 侧把**信封**当**事件**解，导致每条回复都是「无响应」

> 本条是**实现 A–D 过程中新发现的**，不在首轮评审清单里。

**位置**：`plugins/telegram/plugin.rs:540`（旧行号）

```rust
PluginFrame::Data(data) => {
    if let Ok(event) = serde_json::from_value::<NodeEvent>(data) { … }
}
```

**问题**：转写流帧是**信封**——`{type:"transcript_event", data:{session_id, seq, message}}`
（`transcript_stream::publish_frame` 产出，CLI 的 `transcript_frame_of` 与 subagent 的
`node_event_of` 都是「先看 `type`、再取 `data`」）。而 `NodeEvent` 的顶层字段是
`session_id` / `seq` / `message`——信封顶层是 `type` / `data`。

于是 `from_value::<NodeEvent>(data)` **每一帧都失败**（`missing field session_id`），
`full_text` 恒为空，走到 `if full_text.is_empty() { "无响应" }`——**用户侧表现是
「Telegram 机器人永远只回『无响应』」**，且没有任何报错（`if let Ok` 把错误吞了）。

**为什么评审时没抓到**：这条路径在 CLI / Tauri 里都有等价实现且写法正确，
Telegram 是第三份手写副本——**同一段解析写了三遍，其中一遍写错**。

**已修**：改为按信封的 `type` 分派再取 `data`，与另两处逐字对齐。

**留下的教训（结构性）**：帧的解包逻辑当时有三份手写副本
（`cli/client.rs` / `agent/host/subagent.rs` / `telegram/plugin.rs`）。
本次只修了错的那一份，**没有收敛成一处**——因为它跨 crate（CLI 独立工作区），
收敛需要一个住在 `symbio_core` 的公共解包入口。

> **状态：已落地（收敛）**。三份副本已合并为 `symbio_core` 的两个公共入口：
>
> | 入口 | 位置 | 覆盖 |
> |---|---|---|
> | `transcript_stream::event_of(&PluginFrame) -> Option<NodeEvent>` | 与 `publish_frame` 同模块 | CLI、subagent、telegram |
> | `transcript_stream::is_resync(&PluginFrame) -> bool` | 同上 | CLI、subagent |
> | `vdfs_provider::vdfs_change_of(&PluginFrame) -> Option<VdfsChange>` | 与 `VdfsChange` 同模块 | CLI、subagent |
>
> 设计要点：入口住在**产出信封的模块**里，形状改了编译器先响；判别值
> `EVENT_TYPE` / `RESYNC_TYPE` 同时收成常量（原先三处各写裸字面量）。
>
> **顺手修掉同类问题的生产者侧副本**：`plugins/event_bus/plugin.rs` 的
> `connected` 帧曾自己拼 `json!({type:"bus_event", …})`，与
> `event_bus::build_envelope` 是同一形状的两份实现。现 `build_envelope` 改为
> `pub`，插件调用它——**信封形状从此只有一处构建、一处解包**。
>
> 契约用例：`vdfs_change_of` 三条（解信封 / 拒异 kind / 非 `Data` 帧不 panic）+
> 背压标记一条（`event_of` 解不出、`is_resync` 认出——两者不可互相替代）。
> 信封在测试里**手搓**而不调 `build_envelope`：测试要独立于产帧方钉形状，
> 这正是本次事故的形态。

---

## 5. 链路长度评估：哪里真的长

### 5.1 环节数（17）不是问题

见 §1.1 逐项判定。骨架是「执行期原语 → 单一写入点 → 单一帧语义 → 单一落地口」，没有中间层级的冗余。

### 5.2 前端每帧 O(N)：真正的长尾成本

**位置**：`stores/sessions.ts:192-200` + `composables/useChatConnection.ts:189`

```ts
function updateMessages(sessionId, mutate) {
  const next = { ...sessionMessages.value }        // O(S) 会话数
  const cur  = { ...(next[sessionId] || {}) }      // O(N) 该会话消息数  ← 每帧！
  next[sessionId] = mutate(cur) ?? cur
  commitMessages(next)                             // transcriptVersion++
}
```

`messageTree` 也在每帧重建：`childrenMap` 全量重建、**所有 children 数组每帧重新 `sort`**、递归 `buildNode` + `nodeSignature`。

于是单次回复的总成本 = `O(N)` × 帧数 `T` = **`O(N·T)`**。`nodeSignature` 已经把 Vue 侧的**重渲染**降到只改动的路径（这是对的），但**树重建与字典浅拷贝本身**仍是每帧 O(N)。长会话（N 数百~数千）在高速输出时这是主要热点。

**建议**（按性价比排序）：
1. **帧批处理（收益最大、风险最低）**：后端或前端把 ~16ms 内的多帧合成一次 IPC 事件（`{"frames":[...]}`，**每帧保留自己的 `seq`**——因此不违反不变量 #28，缺口检测照常）。IPC 事件数降 2~3×，`updateMessages`/`messageTree` 的调用次数同比例下降。对观感的影响是 ≤16ms 的延迟，不可感知。
2. **字典改增量**：`sessionMessages` 改为 `shallowRef` + 按 `sessionId` 分片（每会话一个 `shallowRef`），delta 帧只触发该会话分片；避免 `{ ...next[sessionId] }` 全量浅拷贝。
3. **排序记忆化**：`childrenMap` 的排序只在「该父节点的子集合发生变化」时重算（可用子 id 列表的签名判定）。

### 5.3 信封过重（协议层）

见 §4.4 与本文的嵌套图：**≈110 字节信封承载 1 字节增量**。绝对值不大（30~100 token/s ≈ 4~12 KB/s），但在「远程 WS 连接另一实例」（`plugin.ts:504-545`）的场景下是实打实的带宽与解析成本。

**建议**（低优先级，需权衡可读性）：
- 把「帧信封」的 `type` 判别改成 `#[serde(tag = "t")]` 的枚举（`e` / `r` / `w`），一层替代两层；
- 短键名（`session_id`→`s`、`message`→`m`、`delta`→`d`）——**但这会牺牲日志可读性，需评估**；
- 更值得做的是**批处理**（§5.2 #1）：N 帧共享一个信封，信封开销被摊薄到 1/N，收益远大于改键名。

---

## 6. 架构优化建议（按优先级）

### 6.1 【高价值】把会话运行态并入转写流，删除第二条实时通道

**现状**：实时面两条通道，各归其域（`node-state-streaming.md` §8 #10）。
- 转写流：消息节点，有 `seq` + resync + 缺口检测；
- VDFS 变更流：会话运行态 + 全部资源变更，无 `seq` + 静默丢帧 + 静默摘除。

**为什么值得合并**：
1. 会话运行态与它的消息**本来就是同一份逻辑状态的两面**（一个节点的 `status` 与它的转写）。现在被拆到两条通道，直接导致 §4.2 的跨通道顺序假设；
2. 合并后，「会话不忙 ⇒ 节点已终态」在**同一 seq 空间内**成立，`reconcileTranscript` 这个补丁可以删除——这正是设计目标「不依赖事件顺序」的完整实现；
3. 连接数从 2 降到 1，重连路径、订阅表、背压策略各少一套；
4. 会话运行态直接获得 `seq` + resync 的强保证，§4.1 的 P0 缺陷**自动消失**（不必单独修 `event_bus`）。

**代价与边界**：
- 资源变更（工作目录文件、agent 目录、skill/mcp 配置…）**不应**并入——它们与某个会话无必然关系，且频率可能很高。合并的**只是会话节点**这一类；
- 需要给会话状态帧一个会话内 `seq`（现成的 `Transcript::emit` 就在分配）；
- `agent/host/subagent.rs` 与 `cli/src/client.rs` 两个进程内消费者要同步改（它们已经同时订阅两条通道，改动是收敛而非扩散）。

**若暂不做**：至少把 §4.1 修掉，并把 §4.2 的假设写进不变量清单。

### 6.2 【中价值】传输层收口

- 统一三种传输的响应容器为 `PluginMessageWire`（§4.5）；
- 统一 `Native` 载荷的处置；
- 把「`Session` 载荷语义」从传输层抽出，让 HTTP invoke 与 Tauri 行为一致（或显式在协议里区分「流式 invoke」与「折叠 invoke」两个语义，而不是同一个 `Session` 两种解释）。

### 6.3 【中价值】扇出与序列化去重

见 §4.4：一次序列化 + `Arc` 共享 + （可选）`RawValue` 直通。

### 6.4 【低价值】清理死代码与未实现字段

`cancel_token`（§4.3）、`actualId` 分支（§4.6）、`kinds`（§4.7）、过时文档（§4.8）。

---

## 7. 建议的落地顺序

| 批次 | 内容 | 风险 | 收益 |
|---|---|---|---|
| **A** | 修 `event_bus` 满通道语义（不摘除 + warn）；补 `reconcileTranscript` 的超时兜底 | 极低 | 消除「会话永久卡在运行中且无信号」的 P0 |
| **B** | 接线或删除 `cancel_token`；删 `actualId` 死分支 | 低 | 去掉「看起来会清理」的假象；订阅表收口确定化 |
| **C** | 前端帧批处理（48ms） | 低 | IPC 事件数与 `O(N)` 重算次数降 2~5× |
| **D** | 后端一次序列化 + `Arc` 扇出 | 中（触及热路径） | 每帧省 1 次 serde 遍历 + N 次深拷贝 |
| **E** | 会话运行态并入转写流（删第二条实时通道） | 中高（跨三端消费者） | 结构性收益最大：删跨通道顺序假设、连接数减半、P0 自动消失 |
| **F** | 传输容器统一 / 协议信封瘦身 / `kinds` | 中 | 长期可维护性 |

**若只能做一件事**：做 **A**。它是唯一一个「用户可感知、且当前无任何自愈机制」的缺陷。

### 7.1 A–D 已落地；E、F 为何未做（附理由，便于下次接手）

> **更新（后续批次）**：**E 已落地**。下面这段"为何未做"保留为**决策记录**——
> 它列出的三条代价（改三端消费者、动 §8 #28 的边界、清单同步要拆）当时都成立，
> 只是权衡的结论在"要的是真正的收益"这一条指令下翻转了。落地结果见
> `node-state-streaming.md` §11，实际改动比预估**更大**也**更干净**：
> VDFS 侧顺带不再携带会话节点快照（那条通道上的快照会与有序通道的状态竞争），
> 于是前端反而**净删**了一套机制（宽限复查 + 整份回读）。
>
> **F 维持未做**，理由见本节末尾——那条不对称是有意的（对外契约 vs 内部统一）。
>
> 另记一笔后续可做的（本次 E 顺带发现，未做）：`VdfsChange` 的 `node` / `content` /
> `delta` 三个载荷字段在**生产代码里已无任何生产者**（E 删掉 `session_change` 之后，
> 剩下的只有测试构造）。它们是真正的死字段，删除属"信封瘦身"，
> 但会动 `docs/design/vdfs.md` 的对外形状与 `useVdfs.ts` 的 `appended` 分支
> （同样已无生产者）——值得单开一批，与 F 无关。
>
> → **已由批次 G 完成**（2026-09-22）：三个无生产者的**变更取值**
> （`renamed` / `appended` / `truncated`）与四个**载荷字段**一并删除，
> 判据立为「一个取值（或载荷字段）必须有生产性生产者」。
> 见 `docs/CHANGELOG.md` 的「变更词汇表收窄为闭集」条目。

**E（会话运行态并入转写流）—— 原判定：「收益真实但本轮不值得」**

建议本身成立，但落地要同时改动**三端消费者 + 一个不变量**：

1. 会话状态帧需要一个与会话内 `seq` **同源**的序号，而序号由 `Transcript` 分配
   （`Transcript::emit`）；把非消息帧塞进同一个计数器，等于改动
   `node-state-streaming.md` §8 #28「`seq` 与发布逐帧无例外」这条不变量的边界。
2. 前端 `stores/sessionNodeSync.ts` 现在从 VDFS 变更取运行态；改为从转写流取，
   等于把「会话清单同步」与「转写同步」两条接线合并——而清单同步还兼职
   资源变更（`created` / `deleted` / 重命名），**不能整条删掉**，只能拆。
3. `agent/host/subagent.rs` 与 `cli/src/client.rs` 两个进程内消费者要同步改；
   新帧类型**没有任何现成测试覆盖**（e2e 的 T9/T10/T11 走的是 gateway WS 边界，
   只覆盖消息帧）。

对比它解决的问题：§4.2 的跨通道顺序假设，其**失败模式是「多一次整份重读」**，
不是数据丢失——重读源（VDFS 读面 = 存储 + 在途）本身就是权威。而本轮已落地的
`reconcileTranscript`（§0.1）把这条假设变成了**有兜底的**：会话离开运行态后宽限期
复查，仍不收敛就整份回读。**顺序假设从「无声失效」降级为「一次可观测的额外读取」。**

因此 E 的价值从「修一个静默缺陷」降为「省一次罕见的重读 + 少一条连接」，
而代价是三端联动重构。**留待有真实性能或故障数据支撑时再做**——那时它才是有依据的
结构改进，而不是「看起来更优雅」。

**F（传输容器统一为 `PluginMessageWire`）—— 未做，判定为「净收益为负」**

- HTTP invoke 的响应形状（`200` + 裸 `PluginPayloadWire`）是**已文档化的对外契约**
  （`docs/design/http-api-transport.md:106`）。改成 `PluginMessageWire` 是**破坏性变更**，
  外部调用方与文档都要跟着改。
- 换来的只是前端 `httpInvokeTransport` 里少一行重新包装（`plugin.ts:494`）——
  而那一行本身就是**归一化**：把「传输层各自的形状」收敛成前端统一消费的
  `PluginMessage`。归一化发生在消费端入口，正是它该在的位置。

**结论**：这条「不对称」是**有意的不对称**（对外契约 vs 内部统一），
不构成缺陷。已在本节留痕，避免下次重复提出。

**F 的两条子项逐条复核（2026-09-22 二次确认，结论未变）**

- **「协议信封瘦身」（§5.3）**：两条建议**都**不成立。
  ①「把帧信封的 `type` 判别改成 `#[serde(tag = "t")]`，一层替代两层」——那两层是
  **运输层**（`PluginFrame` 的 `Data` / `Error`）与**协议层**（`{type, data}`）。
  要压成一层就得让运输层认识 `transcript_event` / `transcript_session` / `transcript_resync`
  这些**协议**判别值，即反向耦合（运输层本该与业务无关）。
  ②「短键名」——收益被 C 批的合帧窗口摊薄（合帧降的是 IPC 事件数，不是字节数；
  而字节数的绝对值在 30~100 token/s 下只有 4~12 KB/s），代价是日志与抓包不可读。
  **更值得做的那条（批处理）已经在 C 批做了**——评审自己也是这么排序的。
- **「传输容器统一」**：见上，破坏性变更换一行归一化，**不做**。

**F 域里唯一真正该改的是文档，已改**：`docs/design/http-api-transport.md` §5.3
（**第三方接入指南**）在 E 落地后已经**与实现不符**——它还在教外部调用方「订阅
`event_bus/subscribe` + `vdfs/watch` 看流式输出」。实际上流式信源自 E 起是
`session/stream` 的 Session 通道，VDFS 只剩会话**资源**变更。该节已整段改写
（含新旧对照与历史说明）。这条比 F 原本的任何一项都重要：它是对外契约的**错**，
而不只是不统一。

`Native` 的处置不一致（同属 §4.5）**已修**——那是真正的语义分叉，且零生产者，
改动无风险。

---

## 附：本次评审确认的事实（供后续引用）

> **S25 复核**：下列事实写于评审当时；标 ⚠️ 的几条**已被后续批次改变**，以行内注记为准。

- 实时面**两条通道**：`worker/session/stream`（转写流）+ `event_bus/subscribe`（`kind=vdfs`）。两者都是 `PluginChannel` + `mpsc` + `DashMap` 订阅表，属机制复用而非双写。
  ⚠️ **S25 起实时面只剩一条**：会话运行态并入 `session/stream`（两种帧、一个 `seq` 空间）；
  `kind = "vdfs"` 只承载**资源**变更。见 `node-state-streaming.md` §11。
- 旧 `kind="session"` 事件频道、`replayBuffer`、`event_bus/pending/snapshot` **已整体废除**，确认无残留。
- `services/vdfsTranscriptSync.ts`、`services/sessionBusWatcher.ts` **已删除**（仅存于 `docs/archive/`）。
- `schemas/vdfs.ts` 的 `sessionRouteOf` **当前生产代码零调用**（仅单测引用）；实际分派走 `session_id` 与地址前缀。
  ⚠️ **S25 已删除**：按地址分派是「消息走 VDFS」时代的解法，消息与运行态都改走转写流后它再无调用方，
  连同 `SessionRoute` 类型一并移除（`architecture-health-check-2026-09.md` F-7 的其中一项据此结清）。
- 后端每帧 **2 次 serde 遍历 + 逐订阅者深拷贝**（`transcript_stream.rs:96-128`）。
  ⚠️ **D 批已修**：`PluginFrame::Data(Arc<Value>)` + 一次序列化，扇出零深拷贝。
- `PluginChannel.cancel_token` 全仓**从未被 `cancel()`**。
  ⚠️ **B 批已修**：`route_connection.rs` / `commands.rs` 已接线，连接断开即触发反注册。
- `ChatMessage` 的可选字段均带 `skip_serializing_if = "Option::is_none"`（`schemas/session/chat_message.rs`），delta 帧本身是瘦的——开销在信封而非消息体。

---

## 附二：本次一并修掉的两个**测试侧**问题（不改协议，但影响可验证性）

### 附二.1 `t10-node-protocol` 是 **50% 概率红**的 flake（已修）

**现象**：断言「进入过非终态的节点都收在终态」失败，时间线显示一个 `turn` 节点
停在 `streaming`。实测 **8 次跑 4 次失败**。

**根因（纯测试侧）**：用例是**两轮**对话（第一轮调工具、结果回灌后再来一轮），
而停止判据是「**任一** Turn 到达终态」+「出现 `role=tool` 帧」——两个条件在
**第一轮**就满足，于是 `ws.close()` 砍掉了第二轮的尾巴。`waitFor` 是 **100ms 轮询**，
轮询间隔内到达的第二轮帧进了 `ops`，它的终态帧没进。

**修法**：换成**静默收敛**判据——「没有任何节点停在非终态」**且**「连续 600ms 无新帧」。
静默窗口是必需的：两轮之间的空隙里「无未终态节点」也可能成立，
只按前者会在空隙处提前收网。修后 **8/8 通过**。

> 这条与 §4.2 是同一类思维错误的两个面：**都试图用一个「弱于断言范围的信号」去判断
> 「可以停了 / 可以认为结束了」。** 协议侧的兜底是宽限期复查，测试侧的兜底是静默窗口。

### 附二.2 `event_bus` 单测在并行下互相投帧（已修）

**现象**：新增 `fan_out_shares_one_payload_allocation` 之后，
`full_channel_eventually_receives_resync_marker` 在并行下随机超时（串行必过）。

**根因**：`SUBSCRIBERS` 是**进程级静态表**，而 `cargo test` 默认并行——
每个用例注册自己的订阅者后，**其它用例的每一次 `try_publish` 也会投进它的通道**。
对「容量 1 + 依赖 Full 触发 resync」的用例，这会把时序搅成非确定。

**修法**：文件内一把 `tokio::sync::Mutex` 串行化（守卫要跨 `.await` 持有，
不能用 `std::sync::Mutex`）。同一张全局表是用例间唯一的耦合面，因此锁只需覆盖本文件。

### 附二.3 新增的两个 `mod tests` 已按约定拆成 `*.test.rs`

`test-layout-audit` 的棘轮是「真内联 `mod tests` 的文件数只降不升」（基线 52）。
本次新增的两处（`event_bus.rs` / `transcript_stream.rs`）已拆为
`event_bus.test.rs` / `transcript_stream.test.rs`，宿主文件末尾改为
`#[cfg(test)] #[path = "…"] mod tests;`，内联数回到基线。
