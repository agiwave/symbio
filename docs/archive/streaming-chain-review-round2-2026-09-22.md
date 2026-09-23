# 全链路评审 · 第二轮（2026-09-22）：大模型服务 / 工具 → 前端

> **文档类型：评审（一次性结论，不是规范）**
> 规范本身仍以 `symbio_core/transcript_stream.rs`（**已删除**，见下方后记 ②）、
> [`session/docs/node-state-streaming.md`](../../symbio/src/plugins/session/docs/node-state-streaming.md)
> 与 [`design/vdfs.md`](./vdfs.md) 为准。本文只记录**某一时刻的观察与判断**。
>
> ⚠️ **后记（S26 / ADR-025，2026-09-23）：本文的两条基线已改变。**
> ① 本文把「转写流 `session/stream`」与「VDFS 变更（`kind=vdfs`）」的**分治**当作既定事实
> 来分析（§2.3 的通道表、§3.6 的「转写流没有服务端会话过滤」等），**那个分治已取消**：
> 实时面迁回 VDFS 变更（`updated` + `delta`），`session/stream` 与 `transcript_stream.rs`
> 退役。② 规范来源里的 `symbio_core/transcript_stream.rs` **已删除**，改以
> [`design/vdfs.md`](./vdfs.md) §9 与 `symbio_core/vdfs_provider.rs` 的「变更通知」小节为准。
> 本文对**性能路径**（一次序列化 / `Arc` 扇出 / 合帧窗口）的实测结论仍然成立，但
> 其中「合帧窗口」这一项随转写流一起退役（`event_bus` 侧的限流与 resync 兜底已对等）。
>
> **与前面两份的关系**：
> [`design/streaming-chain-review-2026-09.md`](./streaming-chain-review-2026-09.md)（第一轮）
> 与 [`streaming-chain-review-2026-09-22.md`](./streaming-chain-review-2026-09-22.md)
> 驱动了 **A–G 七批**落地（满通道语义、cancel_token、前端合帧、一次序列化 + `Arc` 扇出、
> 会话运行态并入转写流、`VdfsChange` 收窄为闭集）。本文在 **E/F/G 之后的新基线**上重做，
> 已落地的项不再重复提出；被判定为「有意的不对称」的项在 §4 二次复核。
>
> **范围比第一轮大**：第一轮只看**出帧**（流式）。本轮按「大模型服务 / 工具 → 前端」把
> **请求方向**与**工具支链**一并纳入——这也是本轮唯一有新发现的方向。
>
> **方式**：读代码（`git grep` 全部出现点），不抓包。每条结论给可核对的 `文件:行`。

---

## 0. 结论摘要

| 维度 | 结论 |
|---|---|
| **是否合理** | **合理。** 请求方向 6 跳全是路由分派，无一跳可省；工具支链**进程内直连**（不经 HTTP/WS）；出帧方向经 D/E/G 三批后已收敛为「一次序列化 + `Arc` 扇出 + 一条有序流」 |
| **是否过长** | **出帧方向不再长**；**请求方向有两处真实的重复物化**（§3.1、§3.2）与一处进程生命周期选择（§3.3） |
| **架构**优化空间 | 集中在**工具支链**：MCP stdio 的进程生命周期（§3.3）、工具结果的形状契约（§3.5） |
| **协议**优化空间 | **小。** §4 三条已判定为「有意的不对称」，本轮复核结论未变 |
| 是否发现**缺陷** | **未发现会改变行为的缺陷。** §3 六项都是「可优化」，不是「错」；其中 §3.1 的代价**比代码注释里估计的高**（重试放大） |

---

## 1. 现状：一轮对话走什么

### 1.1 请求方向（前端 → LLM）：6 跳，跳跳是分派

| # | 环节 | 位置 |
|---|---|---|
| 1 | Tauri IPC `route_v2` → `root.route(ctx)` | `tauri/src-tauri/src/commands.rs:17` / `:63` |
| 2 | `home`（根插件）：终结 `home/*` / `work/*`，其余不剥路径转发 `worker` | `plugins/home/plugin.rs:521` / `:619-626` |
| 3 | `composite`（worker）：取首段 `session` 剥掉，`PATH` 改 `chat/send` | `plugins/composite/composite.rs:271` / `:276-292` |
| 4 | `session`：`"chat/send"` → 一次性发送入口 | `plugins/session/plugin.rs:447` / `:454` |
| 5 | 编排：`tokio::spawn` → 主循环 | `session/orchestrator/entry.rs:169` / `:313`；`consume.rs:98` / `:222` |
| 6 | `run_chat_loop` → `provider.execute_turn` → HTTP POST | `session/chat_loop.rs:76` / `:247`；`plugins/model/bound_provider.rs:78` / `:114`；`symbio_core/turn.rs:219` |

**这里没有冗余**：①→③ 是插件树的**路径分派**（每层只做「剥一段 + 转发」），
⑤ 是异步边界（必须 spawn），⑥ 是真正的协议出口。

值得注意的是 **session → model 不经路由**：`model` 插件的 `route()` 恒返回 `NotFound`
（`plugins/model/plugin.rs:866-870`）。`ModelProvider` 是 core trait，实例在 `traverse`
时注册进 `CapabilityVisitor`，session 取回后**进程内直连**调用。协议差异
（OpenAI / Anthropic / Gemini / Ollama）全部内化在 `BoundProvider.protocol`
一处——这是正确的分层：**协议适配不该出现在路由上**。

### 1.2 请求体被物化几次

一轮 LLM 请求，消息历史被**重建 4 次**、请求体被**序列化 2 次**（无重试时；重试会放大，见 §3.1）：

| # | 动作 | 位置 |
|---|---|---|
| 1 | 从存储重读历史（**每轮整份**） | `chat_loop.rs:152-161` |
| 2 | 请求视图裁剪（淡化 / 骨架化 / 水位） | `chat_loop/inputs.rs:165`（`compression.rs` 内再 `to_vec()` 一次） |
| 3 | 消息树 → 扁平 native | `model/protocols/openai_chat.rs:48-49` → `model/message_builder.rs:42` |
| 4 | 每条消息 → JSON | `openai_chat.rs:52` → `model/types.rs:112` |
| 5 | 拼装请求体 | `openai_chat.rs:55-84` |
| 6 | **为量出字节数再序列化一次** | `bound_provider.rs:102` |
| 7 | reqwest 发送时又序列化一次 | `symbio_core/turn.rs:219` `.json(body)` |

第 6、7 两条是同一件事做了两遍，见 §3.1。

另有 IPC 边界一次往返：前端 `callPlugin` 传对象 → Tauri 序列化 → `commands.rs:44`
以 `serde_json::Value` 存进 ctx → `entry.rs:173` 反序列化成 `Request`。这一层是
**跨界必需**。

### 1.3 工具支链（本轮的重点）

```
LLM 返回 tool_calls（SSE 增量累积为 JSON 文本）
  turn.rs:388 ToolCallAccumulator::process_delta（:430-431）
  turn.rs:456 get_completed（解析成 Value，:472-487；失败记 parse_error）
  ↓
chat_loop/turn.rs:22 取本轮工具批 → :278 process_tool_calls_async
  ↓
tool_executor.rs:746 批量分派 → :249 单工具执行
  :285-309 线上名 → 能力名解析
  :341-372 三分支：命中 → visitor.invoke；未命中/无 visitor → parent.route
  ↓
capability.rs:220-229 invoke_capability（**唯一的「拆信封」点**）
  → Capability::execute(args: Value, env, ctx) -> Result<Value, PluginError>   (:201-206)
  ↓
结果落成 ChatMessage（turn.rs:652 build_tool_message）→ 并入上下文 → 喂回 LLM
```

关键事实：

- **参数是结构化 `Value`，不是字符串**（`capability.rs:224`）。
- **能力返回裸 `Value`，不是 `PluginPayload`**——信封换算只在 `invoke_capability` 一处。
- **执行不跨进程**：`tools.rs:70-81` 同进程 `Arc<dyn Capability>` 方法调用。
  gateway 是**入站**服务，不在工具出站路径上。
- **工具过程对前端可见，只走转写流**：`emit_tool_running`（`tool_executor.rs:587`）、
  结果帧（`:1016` / `:1037` / `:1080`）、父终态（`:616`），统一经
  `orchestrator/sink.rs:47` → `transcript.rs:242` → `:421` 出帧；前端
  `transcriptStream.ts:327` → `:388`。**没有第二条通道。**

### 1.4 出帧方向（后端 → 前端）：已收敛

```
Transcript::apply（唯一写入点 + 唯一 seq 分配）
  → encode：serde_json::to_value 一次      transcript_stream.rs:207-215
  → envelope：Map::insert 移入，不遍历       :196-201
  → fan_out：tx.try_send(frame.clone())     :238-258   ← clone 是 Arc 引用计数自增
  → PluginChannel(4096) → 传输泵 → 前端
```

`PluginFrame::Data(Arc<Value>)`（`transport.rs:30-36`）⇒ **扇出零深拷贝**。
这条是 D 批的收益，实测成立（`transcript_stream.test.rs` 有 `Arc::ptr_eq` 断言）。

前端两遍校验（`plugin.ts:127-140` `validate` + `:145-158` `extract`）**无深拷贝、
无二次 `JSON.parse`**（native 路径）；合帧窗口 48ms（`transcriptStream.ts:182`）。

### 1.5 两条下行通道的分工（E + G 之后的形态）

| 通道 | 承载 | 证据 |
|---|---|---|
| 转写流 `session/stream` | 消息帧 + **会话运行态**帧，共用同一个 `seq` 空间 | `transcript_stream.rs:226-236` |
| VDFS 变更（`event_bus` `kind=vdfs`） | **只**承载会话**资源**变更（创建/删除/改名…），不带载荷 | `vdfs/host.rs:133-137`；`stores/sessionNodeSync.ts:18-29` |

**没有同一事件写两条通道的地方**（逐个 `notify_change` / `sink(...)` 调用点核过）。
唯一「可从两处读到」的是会话节点的全量视图（转写帧 vs VDFS 回读），但
**写入点只有一个**（`session/plugin.rs:125` `session_node_of`）。

---

## 2. 做得对（本轮复核：不宜改动）

| 项 | 为什么对 |
|---|---|
| 工具执行**进程内直连**，不经 HTTP/WS | 省掉一次跨界 serde 往返，也让 `AbortSignal` / `EventSink` 能直接注入（`tool_executor.rs:326-327`）。ADR-020 的实质收益 |
| `Capability::execute` 返回裸 `Value` | 工具从来只用 `Data` 一个变体；信封换算收口在 `invoke_capability` 一处（`capability.rs:210-218` 明写理由） |
| 会话运行态并入转写流（E） | 「不忙 ⇒ 已终态」从**时序巧合**变成**结构保证**（同一 `seq` 空间 + 单通道保序）；前端反而**净删**了一套自愈机制 |
| `VdfsChange` 收窄为闭集（G） | 判据「无生产者的取值不是词汇的一部分」可执行；现在 `useVdfs` 没有快速路径，一律重拉收敛 |
| 出帧一次序列化 + `Arc` 扇出（D） | 每帧 1 次遍历、0 次深拷贝 |
| 前端 48ms 合帧（C） | 降的是 IPC **事件数**（不是字节数），而字节数在 30~100 token/s 下只有 4~12 KB/s——**抓对了量级** |

---

## 3. 可优化项（按性价比排序）

### 3.1 【最划算】请求体在重试循环里被反复序列化

**现状**（`plugins/model/bound_provider.rs:100-102`）：

```rust
// 体量需再序列化一次才能量出（POST 时 reqwest 内部还会序列化一次；
// 请求体在 20KB 量级，这点开销可忽略，换来「实际发出多少字节」这一硬指标）。
let body_bytes = serde_json::to_vec(&body).map(|v| v.len()).unwrap_or(0);
```

然后 `symbio_core/turn.rs:219`：

```rust
get_http_client().post(url).headers(headers.clone()).json(body).send()
```

`reqwest` 的 `.json(body)` 内部就是一次 `serde_json::to_vec`。

**注释的估计少算了一项**：`execute_post_with_abort` 有**重试循环**
（`turn.rs:196`；`const MAX_RETRIES: u32 = 4`，`attempt` 从 0 起、失败后
`attempt += 1` 且 `attempt <= MAX_RETRIES` 才继续 ⇒ **首次发送 1 次 + 最多 4 次重试
= 最多 5 次发送**）。而 `.json(body)` 在**循环体内**，每次发送各序列化一遍。

于是最坏情况是：

| | 次数 |
|---|---|
| 量字节数（`bound_provider.rs:102`） | 1 |
| 实际发送（`.json(body)`，最多 5 次发送） | 5 |
| **合计** | **6**（必要的是 1 ⇒ 多出 5 次） |

不是注释说的「多一次」。20KB 的请求体 ⇒ 最坏约 120KB 的序列化量，
且重试恰恰发生在「网络 / 服务端已经不健康」的时刻。

**改法（约 10 行，无行为变化）**：

1. `let bytes: Vec<u8> = serde_json::to_vec(&body)?`（一次）；
2. `bytes.len()` 就是日志里的体量——而且它现在**必然等于实际发出的字节数**
   （目前只是「理论上应该相同」）；
3. `execute_post_with_abort`（`turn.rs:196`）的 `body: &Value` 改成 `&[u8]`，发
   `.header(CONTENT_TYPE, "application/json").body(bytes.to_vec())`；
   重试复用同一份 `bytes`（用 `bytes::Bytes` 则连 `to_vec` 都省了，只增引用计数）。
   **改动面很小**：该函数全仓只有一个调用点（`bound_provider.rs:114`）。

**风险**：`Content-Type` 要自己设（原来由 `.json()` 代劳）；序列化失败的错误处理
从「reqwest 内部」变成显式。两者都可控。

### 3.2 【中】工具调用参数在「JSON 文本 ↔ `Value`」之间往返 4 次（跨轮）

| 步 | 位置 | 形态 |
|---|---|---|
| 1 流式累积 | `turn.rs:430-431` | JSON **文本**（`entry.arguments.push_str(delta)`） |
| 2 解析 | `turn.rs:472-487` | `Value`（失败记 `parse_error`） |
| 3 落库 | `turn.rs:622-632` | 又变回**文本**（`tc.arguments.to_string()` 存进节点 `content`） |
| 4 下轮请求再解析 | `message_builder.rs:174-180` | 文本 → `Value` |
| 5 发往 API 再序列化 | `types.rs:194-197` | `Value` → 文本 |

根因是**阻抗不匹配**：线上（OpenAI）要求 `arguments` 是 **JSON 字符串**，
而存储里 `ChatMessage.content` 也是字符串，中间却要 `Value` 做校验与归一化。

**可做的（改动量小）**：第 4→5 步是「解析完立刻序列化」，若存储时（第 3 步）
就保证是**规范化后的文本**，则第 4 步可省，直接把文本透传到第 5 步
（`types.rs:194-197` 已有 `if tc.arguments.is_string()` 的快路径）。
代价是放弃一次归一化（键序 / 空白）。

**判定**：中等收益、中等风险（工具参数形状直接影响 LLM 的调用正确性）。
**建议先做的一件事**：给「落库的 `content` 一定是规范化 JSON 文本」补一条断言测试，
再考虑省掉第 4 步——否则省了之后谁也不知道还成不成立。

### 3.3 【中，但触及架构选择】MCP stdio：每次调用一个进程 + 一次完整握手

**现状**（`plugins/mcp/stdio.rs:1-6` 模块文档即明写）：

> 每次调用都 **spawn 新的子进程**，与 MCP server 通信完成后立即 kill。
> 不持有长连接——这是"按需加载（lazy）"设计的一部分。

- 工具**清单**也靠 spawn 拿：`stdio.rs:77` / `:106`（`stdio_tools_list`）。
  而 `mcp/plugin.rs` 的 `traverse`（`:602`）每次都重新注册工具 ⇒ **每次 traverse 都要
  为每个 MCP server spawn 一次进程**，只为拿一份通常不变的工具清单。
- 工具**调用**：`call_tool_stdio`（`:127`）在 `:133` spawn，走完整握手
  （`initialize` + `notifications/initialized`），调用完 `:162` graceful shutdown。

所以一次 MCP 工具调用的固定成本 = **进程启动 + 两次握手往返 + 优雅退出**。
对 Node / Python 实现的 MCP server，进程启动常在 **100–500ms** 量级——
远超调用本身。

**它换来了什么（为什么当初这么选，是有理由的）**：无状态 ⇒ 不留孤儿进程、
server 崩溃不影响下一轮、无需连接池与心跳。这个权衡在**工具调用频率低**
（每轮个位数）时是成立的。

**建议（按可做的顺序）**：

1. **先缓存 `tools/list`**。traverse 时的 spawn 纯粹为了拿清单，与调用无关，
   且清单通常只在配置变更时才变。这是**纯收益、零风险**的一刀。
2. 真要上长连接，就按 server 做**带空闲超时的进程池**，并复用现有
   `shutdown_child_graceful`（`stdio.rs:202-209`）保证退出语义不变。
   这会引入「进程什么时候该死」的新状态——**只在 MCP 工具真的进入高频路径时才动**。

### 3.4 【低】每轮整份重读历史（存储为权威）

`chat_loop.rs:152-161` 在**每轮开头**用 `get_context_messages(None)` 把历史整份读回
并**覆盖**内存镜像；而上一轮收尾时刚做过 `context.messages.extend(tool_results)`
（`chat_loop/turn.rs:288-290`）与父节点整条替换（`:295-302`）——那部分工作
随后就被丢弃。

- **代价**：`O(轮数 × 历史条数)`。50 轮工具循环 × 500 条 ≈ 2.5 万条消息的重复反序列化。
- **但这是刻意的**：它保证请求视图建立在**已持久化**的状态上（与「快照来源必须
  有序或幂等」同源），也让并发写入（用户中途发新消息、压缩）能被看见。

**判定**：**有意的不对称，不是缺陷。** 唯一值得记一笔的是：内存镜像
（`extend` + 父节点替换 + `last_saved` 水位）**只用于驱动 `persist_messages`**，
之后即被丢弃——存在两份表示，但各自的职责清楚。若将来要优化，方向是
「让 `persist_messages` 直接吃增量、内存镜像不再假装是完整历史」，
而**不是**去掉重读。

### 3.5 【低】工具结果的形状靠字段约定猜

`tool_executor.rs:99-122` `extract_result` 按字段顺序猜「给模型看的正文」：

`content` → `output` → `success`（布尔分支）→ 裸字符串 → 整份 `to_string()`

而 `Capability::execute` 的返回类型是 `Result<Value, PluginError>`——**任意 JSON**。

同一文件 `:192` 的注释自己就在说：

> 不是靠猜 JSON 形状——`extract_result` 那种"从任意 JSON 里找 content/output"

即作者已意识到这个问题，并把**控制流**挪到了约定的 `failure_kind` 字段上
（`:95-97` 明写「控制流不得建立在这里」）。所以**当前没有 bug**。

**剩下的风险是「契约没有被写成类型」**：新增一个返回 `{result: "..."}` 的工具，
会静默落到最后一条 `data.to_string()`，把整个 JSON 灌给模型——能通过测试、
看起来也能跑，但不是预期行为。

**建议（低成本）**：
1. `extract_result` 已是**唯一读取点**（全仓仅 `tool_executor.rs:463` 一处调用），
   用测试把「判定顺序即约定」钉死——`tool_executor.test.rs:13-30` 已在做，保持；
2. 把这段字段约定写进 `plugins/session/README.md` 的工具约定段，
   让「工具该返回什么形状」有唯一权威描述处。

### 3.6 【低】转写流没有服务端会话过滤

`fan_out`（`transcript_stream.rs:238-258`）遍历**全部**订阅者，无 `session_id` 判据；
订阅入口 `session/plugin.rs:227-234` 也不接受会话参数（`_ctx` 被忽略）。
消费端自行过滤（CLI `client.rs:374-378`；前端 `transcriptStream.ts`）。

- **当前成本不显著**：前端是**一条**连接（`transcriptStream.ts:472`），
  典型订阅者数 = 1，广播 = 直达。
- **会变成 O(会话数 × 订阅者数)** 的场合：gateway 允许外部 WS 客户端、
  或多窗口并存。真到那时，加一个可选 `session_ids` 的服务端路由即可
  ——但要注意前端是「一条连接看多个会话」，别把它拆成 N 条订阅。

**判定**：**可优化，非缺陷；现在做属于过早优化。**

---

## 4. 已判定为「有意的不对称」（本轮二次复核，结论未变，不再重复提出）

| 项 | 复核结论 |
|---|---|
| **三种传输的响应容器不一致**（IPC 包 `PluginMessageWire` / HTTP 裸 `PluginPayloadWire` / WS 发 `PluginFrame`） | **维持原判定。** HTTP 的裸形状是**已文档化的对外契约**，改成一致是破坏性变更；换来的只是前端少一行**归一化**，而归一化发生在消费端入口正是它该在的位置（`plugin.ts:493-495`） |
| **信封两层**（`PluginFrame` 运输层 + `{type, data}` 协议层） | **维持原判定。** 压成一层就得让运输层认识 `transcript_event` 这类**业务**判别值，是反向耦合 |
| **短键名 / 信封瘦身** | **维持原判定。** 收益已被 C 批的合帧摊薄（合帧降的是事件数不是字节数；30~100 token/s 下只有 4~12 KB/s），代价是日志与抓包不可读 |

---

## 5. 一句话

**出帧方向已经不长了（一次序列化、零深拷贝、一条有序流、闭集词汇）；
真正还有空间的是请求方向，而且最划算的那刀非常小——把请求体序列化一次，
让日志里的字节数和重试复用同一份 buffer（§3.1）。
其余五项都是「现在做不划算、但值得留痕」：工具支链的进程生命周期与形状契约各占一条。
没有发现会改变行为的缺陷。**

---

## 6. 修复进展（2026-09-22）

| 评审项 | 处置 | 提交 / 位置 |
|---|---|---|
| §3.1 请求体只序列化一次 | ✅ 已修复：`execute_post_with_abort` 收 `&[u8]`，`bound_provider` 一次 `to_vec` 量日志 + 传字节，重试复用同一份 buffer；`turn.rs` 兜底 `Content-Type`；新增 e2e `t12-content-type` 钉死 chat/completions 必带 `Content-Type` | `40f6547` |
| §3.2 工具参数 JSON↔Value 往返 | ✅ 已修复：`flatten` 对落库规范化文本以 `Value::String` 透传，下游 `types.rs` `is_string()` 快路径省一次 re-serialize（行为逐字节等价，仅省 CPU）；单测 `tool_call_args_passthrough_as_string_without_reserialize` 钉死 | `770a9ea` |
| §3.3 MCP stdio `tools/list` | ℹ️ 已实现：`discover_tools` 已有 `tools_cache`（TTL + 陈旧 fallback + 配置变更 `invalidate_discover_cache`），评审写于该缓存落地之前；**调用仍每次 spawn 进程属有意设计**（评审明言「除非 MCP 工具真的进入高频路径，否则不动长连接」） | `symbio/src/plugins/mcp/manager.rs` |
| §3.4 每轮整份重读历史 | ⏭️ 有意设计，不改：存储为权威，方向是「`persist_messages` 直接吃增量」而非去掉重读 | — |
| §3.5 工具结果形状约定 | ✅ 已文档化：`extract_result` 判据表 + 常量 + 测试本就钉死顺序；新增 `session/README`「六、工具结果字段约定（跨插件契约）」集中生产方→字段映射 | `eab4887` |
| §3.6 转写流无服务端会话过滤 | ⏭️ 过早优化，跳过：订阅者典型为 1，现在做属过度设计 | — |
