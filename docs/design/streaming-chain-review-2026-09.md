# 流式链路评审（2026-09）：架构、协议与已验证结论

> **文档类型：评审（一次性结论，不是规范）**
> 本文记录对「大模型服务 → 前端」整条流式链路的一次系统评审，含**实测验证**、**发现的问题**
> 与**优化建议**。规范本身仍以
> [`session/docs/node-state-streaming.md`](../../symbio/src/plugins/session/docs/node-state-streaming.md)
> 与 `symbio_core/transcript_stream.rs`（**该模块已于 2026-09-23 随 ADR-025 退役**，
> 会话实时面迁回 VDFS 变更——现行规范见 [`design/vdfs.md`](./vdfs.md) §9 与
> `symbio_core/vdfs_provider.rs` 的「变更通知」小节）为准。
>
> 评审方式：读代码 + 在 **gateway WS 边界**（与 Tauri IPC 同线格式）抓 `session/stream`
> 的 `NodeEvent` 全帧流，逐节点类型核对「开始 / 流增量 / 结束」三态。
> 抓包用例落在 `e2e/cases/t10-node-protocol.mjs` 与 `e2e/cases/t11-compression-node.mjs`。
>
> **后记（S24 续）：** 本文描述的 `NodeOp`（`upsert`/`append`/`remove`/`reset`/`warn`）与
> `NodeChange` 随后被删除——转写流每帧的载荷直接就是一条 `ChatMessage`（`delta` 追加 /
> `content` 整条替换 / `status = removed` 删除），会话级告警下沉为 `TranscriptWriter::warn`
> 独立通道。本文对 `NodeOp` 的论述（§0 结论摘要、§3.2 等）请对照
> [`session/docs/node-state-streaming.md`](../../symbio/src/plugins/session/docs/node-state-streaming.md)
> §6 阅读；评审结论中被 S24 吸收的部分（如告警下沉）已落地。
>
> **后记（S25 / 批次 E）：** 本文 §「问题清单」里关于 `reconcileTranscript` 的几条
> （第 33 行的文档漂移、§4.2 一带「前端没有 `reconcileTranscript`」的推断）**已随批次 E
> 失效**：会话运行态并入 `session/stream` 转写流、与消息帧共用同一个 `seq` 空间之后，
> 「会话不忙 ⇒ 本轮消息已全部落地」成了结构性保证，那张自愈网连同它的
> `applySessionNode` 触发点一并删除；`sessionRouteOf`（按地址分派的实现）也因再无
> 调用方而删除。**本文的结论「跨通道顺序假设是 P0」因此是被正面解决的**，而不是被
> 兜住的——这正是那份自愈网当初要掩盖的问题。详见
> [`docs/archive/streaming-chain-review-2026-09-22.md`](../archive/streaming-chain-review-2026-09-22.md) §7.1。
>
> **再后记（S26 / ADR-025，2026-09-23）：上一条的「结构性保证」本身也被判定为不必要的。**
> 实时面**迁回 VDFS 变更**（`updated` + `delta`），`session/stream` 与
> `symbio_core::transcript_stream` 退役。批次 E 用「单通道 + 共用 `seq`」解决的那个问题
> （跨通道顺序假设），**根本前提是错的**——顺序是**节点属性**（`ChatMessage.seq`）不是
> 投递属性，因此**不存在**跨通道顺序假设这回事。这解释了为什么「跨通道顺序假设是 P0」
> 这条结论在 S20–S26 之间反复出现又反复消失：它每次都在**同一个错误前提**上被重新推导。
> 详见 [ADR-025](../DECISIONS.md)。

---

## 0. 结论摘要

| 维度 | 结论 |
|---|---|
| **协议形状** | ✅ 正确。单一流 + 单调 seq + 显式 `NodeOp`（`upsert`/`append`/`remove`/`reset`），比「补丁语义」高一个量级；丢帧可检测、重放幂等 |
| **节点三态** | ✅ Turn / Reasoning / Text / ToolCall / Compression 五类节点在实测中**全部**具备「开始 → 增量 → 终态」，且增量拼接 == 终态内容 |
| **组合节点终态** | ✅ Turn 终态实测晚于全部子节点终态（§5.3.2 已落地） |
| **链路长度** | ⚠️ 职责分层合理，但**序列化往返过多**：一个 delta 在进程内要经 3 次 JSON 树构建 + N 次深克隆 + 1 次序列化 + 1 次反序列化 |
| **协议根本取舍** | ⚠️ `upsert`/`append` 相对 SSE `delta`+`done`：锚点依赖**已被 `seq` + 重读正确兜住**（取舍，非缺陷）；但**消息面缺"结束"这个一等公民**——尾帧丢失即永久停在 `streaming`（真缺陷）。详见 §2.0 |
| **架构接缝** | ❌ **两条下行通道的接缝是最实质的缺陷**：消息面无法自证"本轮已收敛"，必须靠会话节点通道的时序推测；而那一步自愈在前端**并未实现** |
| **协议面漂移** | ⚠️ `NodeOp::Warn` 是流上的**死变元**；`pending` 是状态机里的**死状态**；用户消息**不带 status** |
| **文档漂移** | ❌ S20.7 声称的 `reconcileTranscript` 在前端代码中不存在 |

---

## 1. 链路：一次 delta 走过的十一段

```
 ① LLM SSE 字节流
 ② symbio_core::turn::parse_sse_stream       按 \n 切行；完整行 serde_json::from_str
    └ 协议层 SseLineParser（含未结束行的 PartialLineExtractor 增量提取）
 ③ ProtocolEvent（ContentDelta / ReasoningDelta / ToolCallDelta）
 ④ dispatch_protocol_event → emit_append / emit_update
 ⑤ EventSink（TranscriptWriter trait → TranscriptSink 实现）
 ⑥ Transcript::apply       内存图整条替换 / 尾部追加 + 分配单调 seq + 一行核心日志
 ⑦ transcript_stream::publish_frame
      serde_json::to_value(NodeEvent)          ← JSON 树 #1
      json!({ type, data })                    ← JSON 树 #2
      for 每个订阅者: tx.try_send(frame.clone())  ← N 次深克隆
 ⑧ PluginChannel（容量 4096）→ 传输层泵（Tauri emit / gateway WS / 进程内直连）
 ⑨ 线格式序列化（IPC / WS）                     ← JSON 树 #3
 ⑩ 消费端 transcript_frame_of / applyNodeEvent   ← 反序列化 #1
 ⑪ store.upsert / append → 渲染
```

### 1.1 分层是否合理

②–⑥ 每一段都有单一且必要的职责，不构成"过长"：

- ② 的「完整行解析 + 未结束行增量提取」双轨是**延迟与正确性的显式取舍**，
  `LineProgress` 记账保证两者不重复计数——不是冗余，是为了让首字节不必等换行。
- ⑤ 的 `EventSink`（出）/ `AbortSignal`（入）取代了「通道兼任执行期协议」，
  使帧里不存在中止帧，是 ADR-020 的实质收益，值得保留。
- ⑥ 的「唯一写入点 + 单调 seq」是整条链路里最有价值的一处收敛：
  上游有十几个发射点，但 seq 与内存图只有一个写入者。

**真正"长"的是 ⑦–⑩**：它们全部只为了跨越"进程边界"这一件事，却对**进程内消费者**
（CLI `cli/src/client.rs`、子智能体转播 `agent/host/subagent.rs`）也照收不误。

### 1.2 优化空间（按性价比排序）

| # | 问题 | 建议 | 收益 |
|---|---|---|---|
| O1 | `publish_frame` 先 `to_value(NodeEvent)` 再包一层 `json!({type,data})`，等于每帧建两棵 JSON 树 | 直接构造**最终 wire 信封**一次（把 `type` 作为 `NodeEvent` 的字段，或用一个专门的 wire struct 一次序列化） | 每帧少一次树构建；热路径逐 token 生效 |
| O2 | 对每个订阅者 `frame.clone()`（深克隆整棵 JSON） | 改成 `Arc<Value>` 或`Arc<str>`（预序列化一次，多订阅者共享） | 订阅者数 × 每帧一次深分配 → 0 |
| O3 | 进程内消费者（CLI / 子智能体转播）也要「序列化 → 反序列化」 | 类型化广播（`broadcast::Sender<Arc<NodeEvent>>` 或回调注册表）与跨进程帧通道**并存**；进程内走前者 | 每个 delta 省 2 次 serde 往返 |
| O4 | 广播给**全部**订阅者，由消费端按 `session_id` 自行过滤 | `session/stream` 订阅时接受可选的 `session_ids` 过滤，服务端按会话路由 | 带宽/CPU 从 O(订阅者 × 全部会话帧) 降到 O(订阅者 × 本会话帧) |
| O5 | `NodeEvent` 用 `#[serde(flatten)]` 承载 `NodeOp` | flatten 走 map 序列化路径，比具名 struct 慢；改成 `op` + `payload` 两段式 | 热路径单帧成本 |
| O6 | `Transcript::apply` 对 `Upsert` 做 `(*message).clone()`（整条 ChatMessage 深拷贝） | 内存图存 `Arc<ChatMessage>`，写时复制 | 终态快照帧少一次深拷贝 |

> O1–O3 是同一件事的三个切面：**"一份事实，多次物化"**。
> 现在每帧至少被物化 3 次（NodeEvent → JSON 树 → 线格式字节 → 消费端对象），
> 而信息量往往只是一个 `&str`。

---

## 2. 协议：逐节点类型的实测结论

实测方法：`session/stream` 全帧抓包（`e2e/cases/t10-node-protocol.mjs`）。
下面是一轮「思考 + 工具调用 + 正文」的真实帧序（节选，`seq` 为流内单调序号）：

```
#1  upsert [text/user]        u-t10                  ← 用户消息（无 status，见 §3.3）
#2  upsert [turn/streaming]   946ab058  meta={turn:0}
#3  upsert [reasoning/streaming] 236be819  parent=turn  content="先"
#4  append ->236be819  "分析"
#5  append ->236be819  "一下"
#6  upsert [tool_call/streaming] e79b244f parent=turn  name=vdfs_write
#7  append ->e79b244f  "{\"path\":\"t10.md\…"
#8  append ->e79b244f  ",\"text\":\"# 写入内容\…"
#9  upsert [text/streaming]   b16c25ab  parent=turn  content="正文甲"
#10 append ->b16c25ab  "正文乙"
#11 append ->b16c25ab  "正文丙"
#12 upsert [reasoning/completed] 236be819  content="先分析一下"   ← 终态 = 增量的收敛
#13 upsert [text/completed]   b16c25ab  content="正文甲正文乙正文丙"
#14 upsert [tool_call/streaming] e79b244f  meta.started_at=…      ← 执行窗口的「运行中」
#15 upsert [text/tool/completed] 19b518e1  parent=e79b244f        ← 结果子节点（成对）
#16 upsert [tool_call/completed] e79b244f
#17 upsert [turn/completed]   946ab058                            ← 组合节点最后收口
```

### 2.0 根本取舍：`upsert`/`append` 与 SSE 的 `delta`+`done`

一个必须先讲清楚的问题：**为什么不用「每帧 delta + 最后一帧 done」这套 SSE 原生形状？**

| | SSE（`delta` + `done`） | 本设计（`upsert` + `append`） |
|---|---|---|
| 增量锚点 | 每个 chunk **自带** id / index | 由**前序 `upsert`** 建立节点 |
| 全量帧 | 无（除非整段重发） | 有：`upsert` 是完整快照，幂等可重放 |
| 结束信号 | **带外**：流 EOF（`[DONE]` + 连接关闭） | **带内**：某个 `upsert` 的 `status` 恰好是终态 |
| 顺序/完整/结束的承担者 | 传输层（一条有序字节流） | 应用帧层（`seq` + 重读） |

由此引出两条批评，逐条回应。

#### 批评一：「`upsert` 变成不可丢失的帧，丢了后面 `append` 就不成立」

**成立，但需要精确定位它的代价。**

- 依赖是真的：`append` 的语义锚定在 `upsert` 建立的节点上。后端
  `Transcript::apply` 对未知 id 的 `append` 直接 `plugin_error!` **丢弃且不占 seq**
  （不造幽灵节点）；前端 `sessions.ts::appendMessage` 同样丢弃并 `logger.warn`。
- 但**代价不是"数据错"，是"触发一次重读"**：`seq` 在发布端严格连续，任何丢失都表现为
  **跳号**，消费端（`transcriptStream.ts::applyNodeEvent`）据此判定"已知有损"并整份重读；
  重读源（VDFS 读面 = 存储 + 在途叠加）本身就是权威，因此恢复**不需要补帧、不需要缓存**。
- 这个依赖**不是 `NodeOp` 引入的，是"窄增量"引入的**。SSE 的 `delta` 也要先知道
  "这段字属于谁"——它把锚点放在**每帧内**（id/index），本设计放在**帧间**。
  真正做到帧间无依赖只有每帧全量，那是 O(n²)，已被明确否决
  （§5.3："此前实现是每片参数全量重发，代价 O(n²)"）。
- **差别在于锚点放帧内还是帧间，代价也就不同**：SSE 每帧多带几十字节 id；
  本设计每帧少带，但换来一次锚点丢失要整份重读。在"每 token 一帧"的热路径上，
  这个取舍偏向本设计——**前提是丢失是罕见事件**。

> 遗留弱点：`seq` 只能暴露"后面还有帧"的丢失。若丢的是**尾帧**，就再也没有后续
> seq 来揭示缺口——这正是批评二。

#### 批评二：「无法有效地知道是否结束」

**成立，且这是本设计最实质的弱点。不是"没做好"，是"没做完"。**

- 消息流里**没有任何一帧表示"本轮结束了"**。终态是一个**普通 `upsert`**，
  与中间态 `upsert` 在帧形状上无从区分；消费端只能"看到某一帧的 `status` 是终态"
  才知道结束。**没到达的东西，消费端无法区分"还没到"与"永远不会到"。**
- 现有的两个结束信号都**不在消息面上**：
  1. 会话节点 `status` 离开 `working`（VDFS watch 域，另一条通道、无 seq、无序）；
  2. CLI 把它当**唯一**结束判据（`client.rs` 注释明写）。
- 于是：**尾帧丢失 ⇒ 节点永久停在 `streaming`，且无自愈**（见 §3.1）——
  这正是"工具已完成、前端一直转圈"的成因。

**SSE 在这一点的优势是真实的**：它的结束是**传输层 EOF**，由 TCP 保证"连接一断就是结束"，
不需要"结束帧不丢"这个假设。代价是它无法跨进程续接、无法重连、无法多消费者。

**所以两条路的真正分野是**：SSE 把「顺序 / 完整 / 结束」三件事**外包给一条有序字节流**；
本设计把它们搬到应用帧层，换来可重连、可多订阅、可跨进程、可按会话过滤，
代价是必须自己重新发明这三件事——**现在发明了前两件（`seq` + 重读），第三件没发明完**。

#### 建议（按改动量排序）

| | 方案 | 说明 |
|---|---|---|
| **A（推荐）** | 转写流增加一个显式收敛帧 | `NodeOp::Sealed { turn_id, seq_watermark }`：由后端在「清在途 → 复位 `is_working` → `emit_session_state`」**之前**发出（该顺序现在已成立，只是没发帧）。消费端据此在**消息面内**判定"结束了且没漏"；到"会话不忙"时仍未见到 `Sealed` 即重读——把现在缺失的自愈变成**可判定** |
| **B** | 会话节点携带消息面 seq 水位 | 不改帧：`attributes.transcript_seq = <本轮最后一条消息面 seq>`。消费端比对自己收到的最大 seq，缺口即重读。把收敛判据**显式写进**会话节点，消除"靠时序推测" |
| **C（最彻底）** | 给流加轮次作用域 | `upsert` 携带 `turn_id`，流上出现 `TurnSealed { turn_id }` 表示该轮全部子节点已终态。"结束"成为一等公民，而不是"等某一帧的 `status` 字段碰巧是终态" |

> A 与 B 可叠加：A 给消息面一个自证信号，B 给跨通道一个可比对的水位。
> 两者都不破坏现有「`upsert` 幂等、`append` 有序」的模型。

#### 一句话

**批评一是对的，但已被 `seq` + 重读正确兜住，属于显式取舍而非缺陷；
批评二是对的，而且是真缺陷——消息面缺一个"结束"这个一等公民。
两者其实是同一个根因的两个表现：帧流有"连续性"（seq），但缺"边界标记"（锚点 / 收敛）。**

### 2.1 五类节点的三态核对

| 节点 | 开始 | 流增量 | 结束 | 实测 |
|---|---|---|---|---|
| **Turn** | `upsert` `streaming` + `meta.turn` | 无（组合节点，自身无正文） | `upsert` `completed`，**唯一发射点** `finalize_turn_root` | ✅ 且 #17 晚于 #12/#13/#16 |
| **Reasoning** | 首片 `upsert` `streaming` | `append` 每片 | `finalize_assistant_turn` 的 `upsert` `completed` | ✅ `"先"+"分析"+"一下" == "先分析一下"` |
| **Text** | 首片 `upsert` `streaming` | `append` 每片 | 同上 | ✅ 三片拼接 == 终态 |
| **ToolCall** | 首片参数 `upsert` `streaming`（带 `name`） | `append` 参数窄增量 | 执行方 `emit_parent_finalized` | ✅ 且 #14 的 `meta.started_at` 覆盖执行窗口 |
| **Compression** | `upsert` `streaming` + 中文占位正文 | 无 | `upsert` `completed`/`failed` + `meta.failure_kind` | ✅ 见 §2.2 |

**协议设计的三个关键正确性，实测均成立：**

1. **帧面只有两种语义**——`upsert`（整条替换）与 `append`（尾部追加）。
   消费端不需要按节点类型猜"这是追加还是替换"，工具参数与正文**同构**。
   这一点在 #7/#8（参数窄增量）与 #10/#11（正文窄增量）上是同一个形状。
2. **增量不丢不重**：`start 快照 + Σappend == 终态 content`（用例 ⑥ 断言）。
   终态是增量的**收敛**，不是另一份独立数据——这消灭了"补丁 + 全量"双轨那一类 bug。
3. **有请求必有响应**：#15 结果子节点（`role=tool`，`parent_id` 指向 ToolCall）
   在流上真实出现且早于父节点终态 #16（用例 ⑪ 断言）。前端响应段有数据源。

### 2.2 压缩节点（T11）

压缩是**根级消息节点**（`parent_id == null`），位置在「本轮用户消息之后、本轮 Turn 之前」，
有自己的开始帧与终态帧，失败时带 `meta.failure_kind`、正文带人读原因。
整表重写后被压掉的消息逐条 `remove` + 快照 `upsert`（`meta.compacted=true`）。
实测两次压缩均符合，且流上的快照 id 与存储一致。

> 这一处设计是对的：把「正在压缩」从会话级横幅改成**有位置的节点**，
> 用户滚到消息流中间也能看见，事后可回溯"上次压缩压掉了多少"。

---

## 3. 发现的问题

### 3.1 【高】两条下行通道的接缝：消息面无法自证收敛

现状：

| 面 | 通道 | 性质 |
|---|---|---|
| 消息实时面 | `session/stream`（`NodeOp` + `seq`） | 有序、增量、丢帧**可检测** |
| 会话运行态 | `event_bus` + `vdfs/watch`（会话节点） | 无序、全量、无 seq |

分治依据（高频突发 vs 低频节点属性）本身成立。但接缝处留了一个**无法在消息面判定**的问题：

> **"本轮的消息节点是否已经全部到达终态？"**

消息流里没有任何"本轮结束"信号（这是 §2.0 批评二的落点）。于是：

- CLI 的 `ask()` 把**会话节点**的 `status` 离开 `working` 当作**唯一**结束判据
  （`cli/src/client.rs` 注释写得很明确）；
- 前端要判定"某个节点是不是卡在 streaming"，也只能等会话节点报"不忙"，
  再去反查消息表——即 S20.7 §③ 描述的 `reconcileTranscript`。

**而 `reconcileTranscript` 在前端并不存在**（`tauri/src/stores/sessions.ts::applySessionNode`
里没有该调用，全仓 grep 无此符号）。也就是说：

> 一旦终态帧在传输中丢失/被跳过（跳号触发的是 `reload`，但**不跳号的单帧丢失不会触发**），
> 前端会**永久显示"运行中"**，且没有任何自愈路径。

这正是 S20.7 §③ 描述的症状（"工具已完成，前端一直运行中"）——**诊断对了，修复没落地**。

**建议（二选一，推荐前者）：**

- **A. 让消息面自证收敛**：在转写流上增加一个显式信号（例如 `NodeOp` 增加
  `Sealed { turn_id }`，或在会话"不忙"前由后端保证发出一帧"本轮消息已收敛"标记）。
  这样消息面不依赖另一条通道的时序，接缝消失。
- **B. 补齐前端 `reconcileTranscript`**：在 `applySessionNode` 的 `wasWorking && !nowWorking`
  处检查本会话是否还有非终态节点，有则整份回读。

> 顺带一提：后端其实**已经保证了顺序**（"节点补丁恒先于会话状态下发"，
> 见 `handle_abort` 的顺序注释）。缺的只是消费端最后一步——这也是它容易被漏掉的原因：
> 顺序成立时看不出问题，只在丢帧时才暴露。

### 3.2 【中】`NodeOp::Warn` 是流上的死变元

生产者（`chat_loop/io.rs` 持久化失败、`turn.rs` 长度截断与工具轮次上限）确实发 `NodeOp::Warn`，
但 `TranscriptSink::apply` **在进 `Transcript` 之前就把它截流**到会话节点 `attributes.warning`：

```
TranscriptSink::apply → NodeOp::Warn ⇒ emit_session_state（会话节点，VDFS watch 域）
                      → 其余        ⇒ Transcript::apply（转写流）
```

于是 `warn` **永远不会成为一帧 `transcript_event`**。但协议面三处都还留着它：

- Rust：`NodeOp::Warn` 变元 + `Transcript::apply` 的防御分支（含 `plugin_warn!`）
- TS：`transcriptStream.ts` 的 `NodeEvent.op` 联合类型含 `'warn'` + `applyNodeEvent` 的 `case 'warn': return`
- 子智能体转播 `subagent.rs:825` 也有 `NodeOp::Warn` 分支

**这不是 bug，是"协议面 > 实现面"的漂移**：三处消费方为一个永不出现的帧写了分支，
下一个人会以为" warn 帧丢了"而去查传输。建议：把 `Warn` 从 `NodeOp` 下沉为
「会话节点专用」的独立类型，或在三处显式标注"此变元不进转写流"。

### 3.3 【低】状态机里的两处空洞

| 项 | 现状 | 影响 |
|---|---|---|
| `pending` 状态 | 状态机 §2.2 定义了 `pending → streaming`，但**实测所有节点首帧就是 `streaming`**，没有节点发过 `pending` | 死状态。前端若为它做了分支，那些分支是不可达的 |
| 用户消息无 `status` | 流上 `text/user` 节点 `status` 恒为 `null`；而压缩产生的快照消息是 `text/user/completed` | 同为 `role=user`，一个有状态一个没有。用户消息不参与状态机，与 §2.2 的表不一致 |

两者都不致命（前端对这两种节点都不靠 status 渲染），但属于**协议面与实现面不一致**，
建议要么补上，要么在状态机表里显式标注"不适用"。

### 3.4 【低】文档漂移

`node-state-streaming.md` §S20.7 声称「前端 `stores/sessions.ts` 新增 `reconcileTranscript`」，
代码中不存在（见 §3.1）。同一节还声称 `emit_message_deleted` 已删除、`truncated` 已引入——
这些在后端成立，前端那一半没跟上。建议：把该节前端部分的描述改为"待实现"或补实现。

---

## 4. 已落地的验证（本轮新增）

| 用例 | 文件 | 验证 |
|---|---|---|
| T10 | `e2e/cases/t10-node-protocol.mjs` | seq 严格递增；`append` 必先有 `upsert`；每节点 `start + Σappend == 终态`；Reasoning / Text / ToolCall 三态；ToolCall 执行窗口带 `meta.started_at`；**Turn 终态晚于全部子节点终态**；ToolCall 必有 `role=tool` 结果子节点且出现在流上；进入过非终态的节点必须收在终态；流与存储同源 |
| T11 | `e2e/cases/t11-compression-node.mjs` | compression 节点 begin(`streaming`+非空正文) / end(终态+非空结果)；失败带 `meta.failure_kind`；根级、早于所属 Turn；重写后 `remove` 目标必须曾在该连接上出现过；流上快照 id 与存储一致 |

两个用例都在 **gateway WS 边界**上抓帧——那是 Tauri 前端 `route_v2` 的同线格式等价物，
因此覆盖的是"前端真正会收到的那一串帧"，而不是后端内部状态。

`mock-llm.mjs` 新增 `reasoning` 场景字段（发 `delta.reasoning_content`），
用于逼出 Reasoning 节点的流式路径（此前只有 `chunks` / `toolCalls`）。

运行：

```bash
node e2e/run-tests.mjs t10      # 单跑
node e2e/run-tests.mjs          # 全量（11/11 通过）
```

---

## 5. 一句话

**协议形状是对的，实现细节是好的，剩下的风险集中在两处：**
**① 消息面不能自证收敛（跨通道接缝 + 前端自愈缺失）；② 每帧被物化太多次（⑦–⑩ 可整体瘦身）。**
其余都是协议面与实现面的小漂移，修起来便宜，但留着会持续误导下一个人。
