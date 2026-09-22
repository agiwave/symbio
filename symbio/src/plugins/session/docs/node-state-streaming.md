# 节点状态流（Node-State Streaming）—— 流模式从「事件序列」改为「节点状态」

> 前置阅读：[vdfs.md](../../../../../docs/design/vdfs.md)（机制规范）、
> [vdfs-session-messages.md](./vdfs-session-messages.md)（转写即列表，S16–S19）、
> [core-loop.md](./core-loop.md)（会话主循环）。
>
> 本文回答一个问题：**会话的"正在发生什么"如何只由节点状态表达**，
> 使前端不再消费任何事件序列，从而**不存在事件顺序问题**。
>
> **状态：S16–S22 已完成；S23 消息实时面改为单条转写流；S24 帧收成「一条消息」；
> S25 会话运行态并入同一条流（批次 E）。**
> 另有 **§10（批次 K）**：转写核心日志的**分级与折行**——**骨架**（出现 / 终态 /
> 等待用户 / 删除）进 `INFO`，**细节**（`Update` 帧与其纯增量的折行统计）进 `DEBUG`；
> 只动日志，`seq` 与发布仍逐帧（不变量 #28）。
>
> - **读面**（历史）走 VDFS：`read(<根>/session/<sid>)` 一次拿整份历史，地址与 §2.1 一致。
> - **实时面**是 `worker/session/stream` **一条流**，两种帧**共用一个 `seq` 计数器**：
>   - `transcript_event`：**每帧就是一条 `ChatMessage`**（`{ session_id, seq, message }`）。
>     **帧里没有独立的操作枚举**——`delta` 追加 / `content` 整条替换 /
>     `status = removed` 就地移除 / 其余字段合并，语义全在字段上（S24 收掉了 S23 的
>     `NodeOp` / `NodeChange`）。
>   - `transcript_session`：**会话节点的全量视图**（`{ session_id, seq, node }`，S25）。
>   由 `symbio_core::transcript_stream` 发布；消费者是前端
>   （`services/transcriptStream.ts`）、子智能体转播（`agent/host/subagent.rs`）
>   与 CLI（`cli/src/client.rs`）。**旧的 `kind = "session"` 事件频道
>   （`Status` / `Update` / `Delete` / `Error` / `Abort`）已整体废除。**
> - **会话运行态**也在这条流上（S25）：它必须与它那一轮的消息**共用 `seq` 空间**，
>   否则「会话不忙 ⇒ 本轮节点已终态」不成立。**VDFS 变更通道只承载资源信号**
>   （创建 / 删除 / 改名 / 标题），**不携带会话节点快照**——快照只有两个来源：
>   这条流（有序）与 `list` / `stat`（回读）。
>
> ⚠️ **§3.1 / §3.2 / §4 / §5 描述的是 S16–S22 的「VDFS 变更」模型**（`created` /
> `appended` / `updated` / `deleted` / `truncated`，消费端
> `services/vdfsTranscriptSync.ts`）。那个文件**已被删除**，消息不再走 VDFS 变更，
> 会话运行态也不再走（S25）——这几节保留为**设计推导的历史记录**，实时面的权威描述
> 在 `symbio_core/transcript_stream.rs` 与 `services/transcriptStream.ts` 的模块文档里。
> **仍然有效**的是：§2（节点分类 / 状态机）、§5.3（工具调用三段式）、§6 的 S20–S25
> 各阶段、§7（体验清单）、§8（不变量）、§11（批次 E）。
>
> §1 的"现状"表同样是当时的问题清单，不是今天的描述。

---

## 0. 一句话

**把「运行中」从事件里拿出来，变成节点的属性。**

前端持有的是一张**按地址索引的节点表**；它唯一消费的是「某个地址的节点变成了什么样」，
按**地址**分派，不按事件类型分派，也不推断"上一条事件是什么"。于是：
**事件可以丢、可以重放、可以乱序，节点状态不会因此错**——因为状态是幂等的全量视图，
而事件是增量的、有顺序的。

---

## 1. 为什么现在还不够

S16–S19 已经把**转写（消息）**整体迁到 VDFS：读走一次 `vdfs/read`，实时走
`kind = "vdfs"` 的 `created` / `appended` / `updated` / `deleted`，载荷带节点视图与内容快照。
但**会话自身的运行态**仍是事件：

| 事实 | 现状通道 | 问题 |
|---|---|---|
| 会话忙 / 闲 | `kind = "session"` 的 `StreamEvent::Status{busy\|idle}` | 事件是增量：丢了就没有第二次机会；`busy`/`idle` 的**顺序**决定 UI 对错 |
| 上一轮以错误结束 | `StreamEvent::Error` + 前端 `last_failed` 平行状态 | 错误被同时表达为「失败节点」与「事件」两处，需要 `hasFailedNode` 这类启发式去重 |
| 用户中止 | `StreamEvent::Abort` | 中止不是错误也不是正常结束，事件里靠"谁先到"区分音色 |
| 连接态 | `Connected` / `Disconnected` | 前端连接态与业务态混在一条通道上 |
| 切会话漏事件 | `event_bus/pending/snapshot` + `replayBuffer` | **这套缓冲机制本身就是"事件会乱序"的补丁**——它存在，说明模型不对 |

`eventBus.ts` 的 `replayBuffer` 注释写得最直白：

> 「旧实现：先订阅，再异步拉 snapshot。副作用：实时事件先到 handler，
> snapshot 中的"更早的事件"反而晚到，造成 Status / Abort 顺序错乱。」

**修法不是把缓冲做得更精细，而是取消"顺序"这个前提**：
让每次变更都携带**该地址的完整状态**，那么"谁先到"只影响最终收敛的**速度**，
不影响**正确性**。

---

## 2. 节点分类与状态机

### 2.1 地址 = 节点

一个节点 = 一个地址 = 一份状态。会话域的全部节点：

```text
<根>/session                                  会话清单（目录）
<根>/session/<sid>                            会话节点        ← 运行态的承载者
<根>/session/<sid>/消息                       转写列表（目录）
<根>/session/<sid>/消息/<mid>                 消息节点（列表项）
<根>/session/<sid>/AGENTS.md                  会话记忆（文件）
<根>/session/<sid>/子会话[/<sub>]             子会话清单 / 子会话节点
<根>/session/<sid>/工作目录[/<rel>]           工作目录树
```

消息节点内部再由 `attributes.parent_id` 组织成树——**树是节点的一个属性，
不是另一套地址**。这样列表（有序、可分页、可增量追加）与树（表达归属）两个需求
用同一份数据同时满足。

### 2.2 节点类型（taxonomy）

| 节点 | `ext` | `type`（`attributes.type`） | 组合？ | 状态取值 |
|---|---|---|---|---|
| 会话 | `session` | — | 否（叶子 + 内部区段之父） | `working` / `active` / `failed` |
| 轮次 Turn | `message` | `turn` | **是**（仅分组，无正文） | `pending` → `streaming` → `completed` / `failed` |
| 思考 Reason | `message` | `reasoning` | 否 | `pending` → `streaming` → `completed` / `failed` |
| 正文 Text | `message` | `text` | 否 | 同上 |
| 工具调用 ToolCall | `message` | `tool_call` | **是** | `pending` → `streaming` → `waiting_user_action` → `completed` / `failed` |
| └ 请求 Request | — | （即 ToolCall 的**内容**） | — | 随 ToolCall：参数流式期 `streaming` |
| └ 响应 Response | `message` | `text` / `turn` | 视内容 | `pending` → `streaming` → `completed` / `failed` |
| 用户应答 UserPrompt | `message` | `user_prompt` | 否 | `waiting_user_action` → `completed` |

**工具调用的「请求 + 响应」不需要两个地址**：`read(.../消息/<tc-id>)` 取到的正文
**就是请求体**（参数 JSON），它的**子节点就是响应**。这与 LLM 协议同构
（`tool_calls[].function.arguments` + `tool` 角色消息），也与既有存储同构。
把请求另立一个地址会立刻产生"同一份参数存两处"，那才是真的坏设计。
前端曾用 `meta.__toolRequest` 凭空合成一个请求节点——**合成的是"节点视图"，
不是"状态"**：它的状态（`streaming`/`completed`）本来就取自 ToolCall 节点。
本次把这条规则写进文档（§5.3），不再让"请求节点"看起来像是另一种东西。

### 2.3 状态机

```text
                    ┌──────────────────────────────┐
                    │  pending  未开始              │
                    └──────────────┬───────────────┘
                                   │ 开始产生内容 / 开始执行
                    ┌──────────────▼───────────────┐
              ┌─────│  streaming  运行中（会话：    │
              │     │              working）      │
              │     └──────────────┬───────────────┘
              │                    │
   需要用户响应│     ┌──────────────▼───────────────┐
              └────▶│ waiting_user_action  等待     │
                    └──────────────┬───────────────┘
                                   │ 用户响应 / 直接结束
              ┌────────────────────┴────────────────────┐
              │                                         │
    ┌─────────▼──────────┐                   ┌──────────▼─────────┐
    │ completed  已结束   │                   │ failed  错误        │
    └────────────────────┘                   └────────────────────┘
```

**会话节点是一个例外（有意为之）**：会话是**长期存在**的，它没有"未开始 / 已结束"。
它的三个状态是 `working`（处理中）、`active`（空闲）、`failed`（上一轮以错误结束）。

| 状态 | 语义 | 用在哪 |
|---|---|---|
| `pending` | 已声明、尚未产生任何内容 | 消息 |
| `streaming` | 正在产生内容 | 消息 |
| `working` | 正在执行（会话的同一件事） | 会话 |
| `waiting_user_action` | 阻塞在用户输入上（审批 / 提问） | 消息 |
| `completed` | 正常结束 | 消息 |
| `active` | 空闲（会话的"无特殊状态"） | 会话 |
| `failed` | 以错误结束 | 消息 + 会话 |

### 2.3.1 为什么**不**把 `streaming` / `working` 合成一个词

它们是同一个概念在两类节点上的两种拼法。合并看起来更"统一"，但代价是：
`status-*` 的 CSS 类名、`isWorkingStatus()` 的判据、以及两处测试断言会一起改，
而**漏改 CSS 类名不会报错、不会失败，只会让流式动画静默消失**——正是本次
「体验不得变差」要防的那类回归。收益（少记一个词）远小于风险。
故：**词汇保持不变**，`isWorkingStatus()` 仍是"会话忙不忙"的唯一判据。

`failed` 是本次**新增**的会话状态，替代 `active + last_failed=true` 这种
"状态 + 平行布尔"的写法——后者要求读状态的人同时读两个字段，
任何一处漏读就静默错；`failed` 单独成态后，"会话是否可重试"= `status == 'failed'`，一处判定。

### 2.4 修掉一处有损映射

`message_status()` 原先把 `Completed` 与「未标注」**都**映射成 VDFS 的 `active`，
文档里记为「无害的有损映射」。它并不无害：消费端必须把 `active` **读回** `completed`
（`vdfsTranscriptSync::messageStatusOf`），即一次信息丢失 + 一次猜测还原。

本次改为：**节点状态就是消息状态**——`pending|streaming|waiting_user_action|completed|failed`
原样透传，消费端不再做任何还原（`active` 只作为旧数据的兜底别名保留）。

> 会话节点的 `active`（空闲）与消息的 `completed`（已结束）不是同一个概念，
> 也不构成"两套词表"：一套词表里 `active` 是"无特殊状态"，`completed` 是"终态"。
> 二者此前被合并成同一个字符串，这才是要修的那一处。

---

## 3. 传输契约：变更 + 载荷

### 3.1 唯一通道

会话域的一切实时变化都经 `kind = "vdfs"` 一条频道（S17 已收敛），
**不存在第二条**。本次把会话节点的运行态也纳入这条频道。

### 3.2 变更类型与载荷

| 变更 | 触发 | 载荷 | 消费者动作 |
|---|---|---|---|
| `created` | 新节点出现（新会话 / 新消息） | `node` + `content` | 插入（**零回读**） |
| `appended` | 节点正文尾部追加（流式 token） | **仅** `delta` | 尾部拼接（**零回读**） |
| `updated` | 状态迁移 / 全量替换（**含会话运行态**） | `node` + `content` | 就地替换（**零回读**） |
| `deleted` | **这一个**节点消失（工具恢复删旧子节点）/ 转写清空 | — | 移除这一项 |
| `truncated` | 该节点**及其之后全部**消失（删除某条消息） | — | 按 `seq` 取「该节点及其后」移除 |

`deleted` 与 `truncated` 是两种语义而不是两种粒度：前者与顺序无关（删完后面还有
父节点的最终回答），后者描述的是一段区间。合并成一个值会让消费者无从分辨，且
截断的代价与历史长度线性相关——见 `vdfs-session-messages.md` §2.2。

**载荷宽度按频率分配**：`appended` 每帧都发 → 只带增量；`updated` 每轮数次 → 带全量。
若 `updated` 不带节点视图，消费者就得回读 `vdfs/stat`——一次状态迁移一次 IPC，
而状态迁移恰恰是**最需要即时**的路径。

### 3.3 会话节点新增的两个场景属性

```text
attributes.outcome = "completed" | "aborted" | "failed"   // 上一轮怎么结束的
attributes.error   = "<面向用户的错误短消息>"              // 仅 failed 时存在
```

- `outcome` 是**状态**（"上一轮的结局"），不是事件：提示音据此选音色，
  不需要"谁先到"来区分中止与失败。
- `error` 挂在会话节点上，是为了覆盖「错误发生在任何消息节点创建之前」这一类
  （能力收集失败、provider 解析失败、transport 级失败）——此时没有任何失败节点
  可承载错误。原先这靠前端 `sessionErrors` 这个**平行状态**兜底，现在它是节点的属性。

---

## 4. 顺序无关性

### 4.1 为什么成立

| 变更 | 幂等？ | 可交换？ | 依据 |
|---|---|---|---|
| `updated`（含会话态） | **是** | **是** | 携带全量节点视图；同一节点的两个状态按任意顺序应用，收敛到较新者 |
| `created` | 是 | 是 | 幂等插入 |
| `deleted` | 是 | 否（与 `created` 竞争） | 需要路径级串行 |
| `truncated` | 是 | 否（与 `created` 竞争） | 同上；区间由**接收方**按 `seq` 算，故不依赖被删节点的通知是否到齐 |
| `appended` | 否 | 否 | 增量语义天然有序 |

因此规则是：**状态类变更按地址应用即可；同一地址上的变更串成一条顺序链**
（`vdfsTranscriptSync::enqueue` 已是既有机制，本次沿用并扩到会话节点）。

### 4.2 残留假设（明确写出，不假装没有）

1. **总线是单条有序通道**。同一条连接上，后端发出的顺序 = 前端收到的顺序。
   跨重连丢帧不产生乱序，只产生"少一次状态"——由下一次 `list` / `stat` 修复。
2. **快照不得降级运行态**。`vdfs/list` 的快照可能比在途变更旧，因此
   「快照只能把 `active` 升级为 `working`，不得把 `working` 降级」这条规则保留
   （`stores/sessions.ts::refreshList` 既有实现）。
3. **`appended` 依赖路径级串行**（既有，不改）。

> 这三条都不是新引入的：前两条是既有的，第三条是 `appended` 的固有属性。
> 本次的净收益是**把「Status / Abort / Error 的全局顺序」这一类依赖整体删除**——
> 它才是 `replayBuffer` 存在的原因。

### 4.3 被删掉的顺序机制

| 机制 | 为什么可以删 |
|---|---|
| `eventBus.ts` 的 `replayBuffer`（切会话防乱序缓冲） | 它只为「按 `sessionId` 订阅的事件流」服务。会话显示不再订阅事件 ⇒ 没有调用方 |
| `fetchPendingSnapshot`（`event_bus/pending/snapshot`） | 同上（后端路由与回放缓冲也已一并废除——它的缓冲靠按 `session_id` 灌入的事件帧填充，VDFS 变更发布方传 `session_id = None`，缓冲永远为空） |
| `sessionBusWatcher` 的 `Status/Abort/Error/Connected/Disconnected` 分支 | 会话态改由会话节点承载 ⇒ 整个模块删除 |

> 「事件可以丢、可以重放、可以乱序」这三句在**状态**上成立，在**增量**上不成立。
> 因此 `appended` 的顺序链（`vdfsTranscriptSync::enqueue`）**保留**——它是增量语义的
> 固有属性，不是顺序假设的残留。删的是「全局状态顺序」这一类依赖。

> **S25 后记（批次 E）**：本节描述的「同一地址上的变更串成顺序链」是 S20–S24 的
> 形态——那时消息仍走 `kind = "vdfs"` 的 `appended` 增量，`vdfsTranscriptSync`
> 是它的消费端。批次 E 把会话域实时面整体收进 `session/stream` 一条流之后：
> **前端不再有增量帧**（`transcript_event` 带的是整条 `ChatMessage`），
> `vdfsTranscriptSync` 与其 `enqueue` 顺序链、`VDFS_CHANGE_APPENDED` 分支一并
> 不复存在；VDFS 上只剩**资源**的粗粒度变更，收敛方式统一为防抖重拉清单
> （幂等 ⇒ 无序无害）。**结论不变的部分**：`appended` 的顺序链仍是增量语义的
> 固有属性——只是会话域现在没有增量了，这条结论由其他追加型资源（日志等）继承。

---

## 5. 前端消费模型

### 5.1 两条输入：一条**保序**，一条**幂等**

S25（批次 E）之后前端只有两个输入：

```text
session/stream（转写流：单通道、会话内单调 seq）
      │
      ▼
  handleStreamFrame(frame)          ← `services/transcriptStream.ts`
      │
      ├─ type = transcript_event   → 消息帧：进合帧窗口 → sink.applyMessageBatch
      ├─ type = transcript_session → 运行态帧：先 flushPendingFrames(sid)
      │                              → sink.applySessionState(sid, node)
      └─ type = transcript_resync  → 背压标记：丢本地缓存、整份回读

kind = "vdfs" 变更（**独立无序通道**）
      │
      ▼
  subscribeVdfsChanged({ prefix: <根>/session, directChildren: true })
      │                              ← `stores/sessionNodeSync.ts`
      ├─ deleted                    → sink.removeSessionLocal(id)   （即时）
      └─ created / updated / renamed → 防抖重拉清单                  （800ms）
```

**为什么不再需要「按地址分派」**：那一套（`sessionRouteOf`：地址 → `session` /
`messages` / `message` 三种目标）是消息走 `kind = "vdfs"` 时的解法——它让消费端
**不依赖顺序**。批次 E 换了个解法：两种帧都携带**全量节点视图**、共用**同一个
`seq` 空间**、走**同一条通道** ⇒ 顺序由结构保证，状态本身幂等，消费端同样不必
记历史。（`sessionRouteOf` 因此再无调用方，S25 一并删除。）

这条换法只对**状态**成立：增量帧（`appended`）不可能不依赖顺序。所以会话域
现在**没有增量帧**——消息帧带的是整条 `ChatMessage`（`delta` 由接收端累加，
`content` 整条替换）。热路径的代价从「解地址」变成「多带一点载荷」，换掉的
是一条跨通道顺序假设。

VDFS 那条通道上剩下的是**资源**变更（创建 / 删除 / 改名 / 标题 / metadata），
它们发生在**没有在途轮次**时，拿不到 `seq`，因此必须留在这里；收敛方式统一为
**重拉清单**（幂等 ⇒ 无序无害），`updated` 的载荷不再被解读（见
`stores/sessionNodeSync.ts` 模块文档「为什么这里的变更不带节点快照」）。

两个消费者**互不重叠**：会话叶子的**运行态**只归转写流的运行态帧，**资源**只归
VDFS 订阅，**转写**只归转写流。重叠会让同一份状态被写两次。

### 5.2 状态 → 视图，不做推断

| 视图 | 唯一依据 |
|---|---|
| 会话卡片状态点 | 会话节点 `status` |
| 会话卡片"上次失败" | 会话节点 `status == 'failed'` |
| 会话卡片"等待审批" | **派生**：转写中存在 `status == 'waiting_user_action'` 的节点（纯函数，不是事件） |
| 活动文字（"正在思考…"） | **派生**：最近变化的节点的 `status` + `type` + `tool_name`（工具名的 attributes 键**不能**叫 `name`——那与 `VdfsNode.name`（节点 id）在 flatten 序列化下同名冲突，见 `message_node` 注释） |
| 消息块外观（流式动画 / ⚠ / 骨架） | 消息节点 `status` |
| 提示音 | 会话节点 `working → 非 working` 的**迁移** + `attributes.outcome` |
| 错误条 | 失败节点（有则用节点）；无失败节点时用会话节点 `attributes.error` |

「派生」是**纯函数**：同样的节点表必然得到同样的视图。
事件流做不到这一点——它必须知道"之前发生过什么"。

### 5.3 工具调用的三段式

渲染仍是「请求 / 过程 / 结果」三段，但每一段的**状态来源**写死：

| 段 | 数据来源 | 状态 |
|---|---|---|
| 请求 | ToolCall 节点的 `content` | `streaming`（参数仍在流式）/ `completed` |
| 过程 | ToolCall 的子 `turn` 节点（子会话过程） | 子节点自身状态 |
| 结果 | ToolCall 的其余子节点 | 子节点自身状态 |

`meta.__toolRequest` 保留为**渲染标记**（表示"这是一个由内容提升出来的请求视图"），
但它**不再是状态来源**——状态一律读 ToolCall 节点。

**请求那段的内容是窄增量下发的**：ToolCall 的参数与正文 / 思考**同构**——首帧
（身份字段出现：新节点 / 首次定名）是完整快照，其后每片参数只发 `append`。
此前实现是"每片参数全量重发"，代价有两处：线材开销随参数长度 **O(n²)** 增长；
消费端则被诱导去按"`tool_call` 的帧都是全量"猜语义——那条猜测一旦蔓延到
`role = tool`，就会把工具响应**真正的增量**也当成全量替换（详见 §8 #27）。

### 5.3.1 ToolCall 的 `streaming` 覆盖「参数流式 + 执行」两段（S20.1）

§2.2 的状态机写的是 `pending → streaming → waiting_user_action → completed / failed`，
其中 `streaming` **同时覆盖两个阶段**：

```text
  pending ──▶ streaming ────────────────────────────────▶ completed / failed
              │                    │
              │ ① 参数流式          │ ② 执行窗口
              │   （模型逐 token     │   （一次编译 / 一次网络请求 /
              │     吐出 arguments） │     一个子智能体跑完 —— 往往最长）
              └────────────────────┴──────────────────────
```

**这两段之间没有中间态**，也不该有：对用户而言"这次调用还没结束"是同一件事。

#### 为什么必须有 ② 的状态

`finalize_assistant_turn` 在 LLM 流结束时天然是"参数齐了"这一刻。它**曾在此把所有
ToolCall 标 `Completed`**——于是前端在执行窗口内没有任何「运行中」迹象：参数流完
画面静止，直到结果突然出现。窗口越长的工具（语义索引、长命令、子智能体），
这段静默越长，用户无法判断"还在跑"还是"卡死了"。**这是 S20 交付时留下的一个
显示层缺口，S20.1 修掉。**

#### 不变量：ToolCall 的终态只由执行方给出

| 执行路径 | 终态来源 |
|---|---|
| 正常分发 | `tool_executor::process_tool_calls_async` |
| 恢复执行（approve / retry / supply） | `resume::process_tool_resume_action` |
| 被 PreToolUse 拦下 / 用户中止 / 交互中断 | `process_tool_calls_async` 末尾统一收口：父节点 `Completed` + `meta.failure_kind = "not_executed"`，并由 `not_executed_result` 补一条结果子节点（**成对**，缺一就是「有请求无响应」） |

**"本批每一个 ToolCall 都必须以终态收场"** 由 `process_tool_calls_async` 负责保证：
调用前广播 `Streaming`，调用后广播终态，未执行的批尾在函数末尾一次性收口。
漏掉任何一条，前端就会有一个**永远转下去的「运行中」**（比"没有迹象"更糟：
它把"卡住"伪装成"在跑"）。

#### 不变量：有调用必有结果（终态之外的第二半）

终态只说"这次调用结束了"，而"这次调用的结果"在树里是一条**子节点**——
前端 `ToolCallNode` 的响应段就按子节点渲染。因此收口同样必须**成对**：

> 每一个 ToolCall 都必须有结果子节点（`role = tool`，`process_tool_calls_async`
> 的 `tool_messages` 里一条），无论它是执行了、失败了、还是根本没轮到。

只补父节点是一个**静默**的缺口（实测事故）：卡片有请求、没有响应，
而会话照常往下走；下一轮请求又因为 `flatten_chat_messages` 会为无结果的
ToolCall 合成占位 tool 结果而**始终合法**——于是「模型看得见、用户看不见」，
没有任何机制会纠正它。因此结果子节点必须在**存储**里成立
（`not_executed_result`），而不是只在请求视图里成立。

#### `meta.started_at`：把"运行中"从断言变成判据

`Streaming` 状态只回答"在不在跑"，不回答"跑了多久"。因此执行开始时同时写入
`meta.started_at`（毫秒），前端据此显示 `运行中 · 47s`：

- **为什么是节点属性而不是前端计时**：切会话 / 重连后前端计时从零重来，会把
  "已经跑了 3 分钟"显示成"刚刚开始"——恰好丢掉用户最需要的那条信息；
- **为什么前端在锚点缺失时不显示时长**：宁可不说，也不给一个看起来合理但
  没有依据的数（旧数据没有该字段）；
- 只写 `started_at`（一个不可变的事实），**不写** `running: true` 这类布尔——
  布尔在结束时需要有人负责清除，漏清就是永久误报。

> 前端另有一条独立收益：`syncActivity` 原本会在 `finalize_assistant_turn` 的
> `Completed` 到达时把活动文字清空，执行窗口内因此也是空的。运行态跨越执行窗口后，
> 活动文字（`正在调用 <name>…`）与节点标签自然同步，无需额外接线。

### 5.3.2 组合节点的终态**跟随子树**（Turn 与 ToolCall 同一条规则）

`turn` 与 `tool_call` 都是**组合节点**（自身无正文，只负责分组），因此共用一条规则：

> **组合节点的终态不得早于它的子树。**

对 ToolCall，这条规则落在 §5.3.1——终态只由执行方给出（"参数齐了" ≠ "调用结束了"）。
对**根 Turn**，它落在**收尾点**：

| 层 | 位置 |
|---|---|
| `chat_loop.rs` | `close_turn` 返回之后调用 `finalize_turn_root` —— 全流程**唯一**一处 |
| `chat_loop/io.rs` | `finalize_turn_root`：从权威转写取那条节点 → 改状态 → 广播 |

它此前在 `finalize_assistant_turn`（LLM 流结束那一刻）发出，比子树早了整整一个
**执行窗口**：实测抓包里「Turn 已完成」出现在「工具开始执行」之前（seq 5 < seq 6），
树上于是出现"容器已经完成、其中的工具调用仍在运行"这种自相矛盾的形状。

**发出的就是即将落库的那条节点**（`build_assistant_messages` 的产物，与
`persist_messages` 同源），因此**实时帧与存储按同一取值收敛**，不需要第二套
"终态"构造逻辑。（副作用：实时流里那条快照不带流式占位期的 `meta.turn`——
存储本来也没有它，两边一致。）

---

## 6. 迁移阶段

### S20 —— 会话运行态上节点（本次）

| 层 | 改动 |
|---|---|
| `session/plugin/nodes.rs` | `message_status` 不再坍缩 `completed`；新增 `SessionRuntime`（含 `from_state` 单一投影入口）+ `session_node(s, runtime)`（运行态投影到 `status` / `attributes.outcome` / `attributes.error`）+ `session_change` |
| `session/active.rs` | `ActiveSessionStateInner` 增加 `last_outcome: Option<String>`、`last_error: Option<String>` |
| `session/orchestrator/broadcast.rs` | `broadcast_status(status)` → `emit_session_state(SessionStateChange)`：写运行态 → 发**带 `node` 载荷**的会话节点 `updated`；不再广播 `Status` 帧（旧事件频道已随 S22 整体废除）。**S25 起第 2 步改走转写流，见 §11** |
| `session/orchestrator/{consume,entry,orchestrator}.rs` | 全部改调 `emit_session_state` |
| `session/plugin.rs` | `notify_change` 旁增 `notify_session_state`（带节点视图）。**S25 起删除**：VDFS 侧只剩 `notify_change` 的粗粒度信号 |
| `session/plugin/vdfs_provider.rs` | `is_working(id)` → `session_runtime(id)`（`list` / `stat` / 变更三处同源） |
| 前端 `schemas/vdfs.ts` | 补状态词常量；`parseTranscriptPath` → `sessionRouteOf`（含会话叶子）；新增 `sessionRuntimeOf` / `chimeKindOfOutcome` |
| 前端 `stores/sessions.ts` | `syncSessionNode`（回读）→ `applySessionNode`（**零回读**）：状态 / 结局 / 标题 / 错误就地收敛，状态迁移触发提示音；`last_failed` 布尔删除。**S25 起改名 `applySessionState` 并改由转写流驱动，见 §11** |
| 前端 `services/vdfsTranscriptSync.ts` | 分派改 `sessionRouteOf`；`messageStatusOf` 不再把 `active` 猜回 `completed`（仅作旧数据别名） |
| 前端 `components/ModelChatPanel.vue` | 错误条改由**节点表派生**（有失败节点则隐藏），不再在事件到达时判定 |
| 前端 `services/sessionBusWatcher.ts` | **删除**（连同 `MainLayout.vue` 的接线） |
| 前端 `services/eventBus.ts` | 删除 `replayBuffer` / `fetchPendingSnapshot`（失去唯一调用方） |

### S20.1 —— ToolCall 运行态跨越执行窗口（本次）

S20 把**会话**运行态搬到了节点上，但**工具调用**的运行态还断在执行窗口之外
（`finalize_assistant_turn` 提前定格）。本次补齐：

| 层 | 改动 |
|---|---|
| `session/chat_loop/state.rs` | `finalize_assistant_turn` **不再**在 LLM 流结束时把 ToolCall 标 `Completed`（参数齐 ≠ 调用结束） |
| `session/tool_executor.rs` | 新增 `emit_tool_running`（执行前 `Streaming` + `meta.started_at`）与 `not_executed_patch`；被拦下 / 中止 / 交互中断的批尾在函数末尾统一收口 |
| `session/resume.rs` | approve / retry / supply 三个真正重跑工具的 action 在执行前同样置 `Streaming` + `meta.started_at`（reject / answer 不执行工具，不置） |
| 前端 `composables/useRunningClock.ts` | **新增**：全应用共享的秒级时钟，引用计数归零即停表（`MessageNode` 是递归组件，每实例一个定时器会线性增长） |
| 前端 `components/MessageNode.vue` | 状态标签覆盖全部非终态（`运行中` / `待确认` / `失败`）；运行中带三点脉动 + 已运行时长；`headClass.thinking` 扩展到工具调用（标题呼吸） |

### S20.2 —— 删除的两种语义分开，级联由前端自己算（本次）

S20/S20.1 处理的是「状态怎么到节点上」，本次处理一个一直存在的**语义混淆**：
`deleted` 同时被用来表达「删这一个节点」和「从这里删到末尾」，前端无从分辨。

| 层 | 改动 |
|---|---|
| `symbio_core/vdfs_provider.rs` | 新增 `VDFS_CHANGE_TRUNCATED`：`path` 所指节点**及其之后全部**已移除 |
| `session/plugin.rs` | `emit_message_deleted`（逐条）**删除**，改为 `emit_transcript_truncated`（一条）；逐节点删除的唯一来源是 `resume` 的 `MessageChange::Remove`（消费循环直接转译） |
| `session/handlers.rs` | `invoke_delete_message` 由「逐条发 `deleted`」改为「发一条 `truncated`」；目标不存在时不发任何变更 |
| 前端 `schemas/vdfs.ts` | 补 `VDFS_CHANGE_TRUNCATED` |
| 前端 `stores/sessions.ts` | 新增 `removeFrom`（按 `seq` 取「该节点及其后」）；`deleteMessage` 改为**本地先行级联** + 失败回滚 + 用权威 `deleted_ids` 幂等对齐；`removeMessageById` 的存在性检查提到对象展开之前 |
| 前端 `services/vdfsTranscriptSync.ts` | `truncated` → `removeFrom`；`deleted` 仍只删一项（工具恢复依赖它） |

为什么不是「在 `deleted` 上挂 `cascade` 布尔」：那会让「是哪种删除」变成两个字段
必须一起读才正确，正是本仓库反复否决的「状态 + 平行标志位」。见
`vdfs-session-messages.md` §2.2。

### S20.3 —— 中止收口与在途可见性（本次）

S20~S20.2 都假设「节点状态会自己走到终态」。本次处理两个**同源**缺口——它们都
出在「存储里的权威副本」与「正在跑但还没落库的那一份」不一致时：

1. **在途消息不可见**：assistant 侧节点**每轮结束**才落库（`chat_loop::persist_messages`），
   而会话叶子 `read` 只序列化存储 ⇒ 前端 `loadMessages`（读叶子）在流式期间看不到
   正在跑的那一轮；切走再切回，在途消息凭空消失。
2. **中止后节点停在「运行中」**：非流式工具执行（`PluginPayload::Data` 分支）原本是一次
   裸 `await`，既不轮询通道也不看中止标志；`handle_abort` 的 3s 兜底一旦触发就会强制
   `is_working = false`，而当时"下一帧"才是循环的唯一醒来时机 ⇒ 消费循环因 `!is_working`
   **直接 `break`，跳过 `persist_failure`** ⇒ 工具节点永远停在 `Streaming`。
   这正是「终止后还显示运行中」。
   （S20.3 由 `wait_tool_abort` 补上第三个出口；批次 E 起该函数收敛为一行
   `abort.cancelled().await`，消费循环亦不再"按帧醒来"——见
   [core-loop.md](./core-loop.md) §8。）

| 层 | 改动 |
|---|---|
| `session/orchestrator/failure.rs` | 新增 `is_inflight`（在途集合的**唯一**定义）与 `converge_inflight`：收敛**在途缓冲**中尚未落库的非终态节点，定稿并广播补丁，**幂等**（存储由写入不变量保证只有终态，无需扫描） |
| `session/orchestrator/consume.rs` | 循环出口记 `exit_state`：中止出口发 `aborted`（此前一律 `completed`，与 `handle_abort` 抢同一个字段，收敛靠 3s 轮询的时序侥幸） |
| `session/orchestrator/consume.rs` | `handle_abort` 复位 `is_working` 后**自己**调 `converge_inflight`——中止路径自己的收口责任，不等 chat_loop |
| `session/tool_executor.rs` | 新增 `wait_tool_abort`：`route_fut` 与它 `select!`，让**非流式**工具也能被中止（当时的三条来源——abort 帧 / 取消令牌 / 已置位标志——批次 E 起归一为共享的 `AbortSignal`） |
| `session/resume.rs` | 重跑工具时中止：父 ToolCall 已落库且置 `Streaming`，返回前用 `not_executed_patch(.., "aborted")` 就地定稿并广播 |
| `session/plugin/nodes.rs` | `session_content` 叠加 `live`（与 `transcript_of` 同源的 `overlay_live`）——叶子与转写列表必须是同一份消息集合 |
| `session/plugin/vdfs_provider.rs` | 叶子 `read`（含子会话）取 `live_messages_of`；子会话叠加**它自己**的在途缓冲，不是父会话的 |
| 前端 `stores/sessions.ts` | `hydrateFromHistory` 改**合并**语义：快照权威；快照里没有的本地节点**仅当仍在飞行中**才保留 |

**为什么在途集合不含 `WaitingUserAction`**：它不是「正在跑」，是「等用户回答」——
中止时抹掉它等于把审批入口删了；它也不会让前端显示"运行中"（前端对它有独立的
「待确认」标签）。反之漏掉 `Pending` / `Streaming` 中的任何一个，表现就是一个
永远转下去的「运行中」，而它**不报错、只会一直转**——因此这个集合由测试直接锁定
（`orchestrator.test.rs::inflight_set_covers_..`），不靠读代码。

**为什么 `converge_inflight` 只看在途缓冲**：`Streaming` 是瞬态状态，持久层写入不变量
（`ensure_durable_states`）拒绝它落盘——「存储里的在途节点」按设计不存在。在途缓冲是
唯一可能停在瞬态状态的地方。（历史上这里扫过存储 + 在途两个数据源，那是围绕
「resume 会把 `Streaming` 落库」这一错误假设的补丁；该假设已随写入不变量的建立被推翻，
扫描分支连同其测试造数一并删除。）

**为什么 `wait_tool_abort` 忽略通道关闭**：对端消失（chat_loop 已返回）不是中止信号。
把它当中止会让**每一次正常收尾**都变成"用户中止"，于是正常完成的会话被报成
`aborted`、提示音选错音色。这条曾写错并被回退测试抓到。
（批次 E 起这条已无实现可误读：`wait_tool_abort(abort)` 只等 `abort.cancelled()`，
**没有通道可关闭**，因此"对端消失"与"用户中止"在类型层面就是两件事。）
（`tool_executor.test.rs::wait_tool_abort_ignores_closed_channel`）。

### S20.4 —— 压缩作为消息流中的节点（本次）

S20.3 修的是「节点已经存在但状态没收敛」。还有一个更靠前的空档：**节点根本还没
被创建**。

自动压缩（`apply_compaction`）跑在每轮 Turn **创建之前**，而它是一次完整 LLM 请求
——长上下文时**可达数分钟**。这段时间里后端刻意静音了全部出帧
（`send_compression_request` 用哑 `tx` 接住压缩 delta，以免泄漏一个永不 finalize 的
空 Turn 骨架），于是**没有任何消息节点**可供前端渲染：用户视角就是"点了发送、
界面毫无反应"。

**先前的做法（已废弃）**：把"正在压缩"作为会话节点的 `attributes.phase`，前端在主
聊天区顶部挂一条横幅。它能工作，但横幅**没有位置概念**——压缩本就是会话里发生的
一步，用户滚到消息流中间时看不见它，事后也无法回溯"上次压缩压掉了多少"。

**现在的做法**：压缩是一个**消息节点**（`msg_type = compression`），与工具调用并列。

| 层 | 改动 |
|---|---|
| `symbio_core/.../chat_message.rs` | `MessageType` 新增 `Compression`（**非对话内容**，是系统对历史的一次整理动作） |
| `session/chat_loop/state.rs` | `CompressionEmitter`（原 `PhaseEmitter` 改名换职责）：`begin` 插入在途节点并广播，`finish` 定稿并返回终态 |
| `session/chat_loop/compress.rs` | 包装层 `compress_with_snapshot_core` 管节点生命周期，结束后 `append_messages` 落库 |
| `plugins/model/message_builder.rs` | `flatten_chat_messages` **剔除**压缩节点 |
| 前端 `components/MessageNode.vue` | 图标 / 标题 / 状态标签 / 正文分支；运行中复用工具调用的脉动动效 + 已用秒数 |

**为什么能在静音窗口里发出去**：`emit_message_patch` 走 VDFS 变更订阅，
**不经过**被静音的那条 turn channel。
所以不是"发不出"，是"从没发过"。

**为什么还要进在途缓冲**：会话叶子 `read` 会叠加在途（`overlay_live`）。不进的话，
压缩期间切走再切回就只剩"什么都没有"，又变回最初的卡死观感。

**为什么落库**：压缩是会话里真实发生的一步，应当留下记录。否则用户刷新后只看到
"历史突然变短了"，却没有任何东西说明发生过什么。

**为什么剔除出请求包**：既白占上下文，又会让模型把"系统整理过上下文"当成一条事实
陈述读进去。

### S20.5 —— 压缩请求改为「历史对话原样 + 末尾指令」（本次）

旧写法 `build_compression_request` 是 `serde_json::to_string(history)`，把整个
`Vec<ChatMessage>` 塞进一条 user 消息。每条消息都带着 `id` / `parent_id` / `seq` /
`status` / `meta`（token 统计、工具名、转存路径……）——这些是**存储与前端**需要的，
模型一个都不关心，却要为它们的 JSON 语法原样付费：字段名、引号、转义、嵌套括号
全是纯开销，且随历史长度与消息条数线性放大。

改为把历史消息**原样**交给 provider：它的 `flatten_chat_messages` 会把消息树投影成
模型熟悉的对话形态（`Turn` → assistant 聚合、`ToolCall` → tool_calls、工具结果 →
`role=tool`，并裁掉陈旧思考链）。模型看到的是**对话**，不是数据转储。

附带两处修正：

- 快照校验失败的纠正重试（`context.messages.push(retry_msg)`）天然变成一段正常的多轮
  对话（历史 → 指令 → 纠正），而不是"JSON 之后追加一句话"；
- 超限预判的 `pending_tokens` 不再把**根本不发往模型**的 `keep_messages` 算进请求体。
  高估会让本可成功的 LLM 摘要被误判成"注定超限"，退化成机械兜底（快照质量显著下降）。

压缩**指令**的措辞刻意不提 `<state_snapshot>` 标签名：结构定义只在 system 侧出现
一次，在对话里重复格式指令会诱导模型模仿模板而非按 system 输出。

### S20.6 —— 压缩失败的**可诊断性**与**熔断**（本次）

实测会话 `09d74431…`（长 GUID，见下）出现过**两次**压缩失败，相隔约 4 分钟，两条
`compression` 节点都是 `Failed` 且 `meta = None`、`error = None`——连失败原因都没留下。
这意味着：① 用户每发一条消息都要白等一次数分钟的**注定失败** LLM 请求；② 事后完全无法
判断是 LLM 请求失败 / 快照校验失败 / 兜底无收益。两个缺陷都修。

**失败原因必须可诊断**（任务 26）：

- `compress_snapshot_inner` 旧逻辑所有失败出口统一 `return None`，调用方只知"失败"不知
  原因。`CompressionFailure` 枚举把出口细分为 `Llm(String)`（带 provider 错误文本）/
  `InvalidSnapshot` / `InputOverLimit`，由包装层经 `CompressionEmitter::finish(node, status, text,
  failure_kind)` 写进节点：`error` 装人读原因、`meta.failure_kind` 装机读码（与未执行工具节点
  同一字段约定）。
- 自动路径的 `auto_compress_process` 改为 `Result<Option<usize>, CompressionFailure>`
  透传；手动 `run_context_compact` 同样用 `message()` 回写节点。

**连续失败必须熔断**（任务 27）：

- 根因：`auto_compress_process` 只看当前 token 水位，不记得"上次失败"。压缩失败→上下文
  **原样回滚**→下轮仍超阈值→再压→再败——死循环，且每次都是一次完整的 LLM 往返。
- `ActiveSessionStateInner` 新增 `auto_compress_failures: u8` 与
  `auto_compress_circuit_opened_at: Option<Instant>`，由三个 `async` 辅助方法集中维护
  （`compression_should_skip` / `compression_record_success` / `compression_record_failure`）：
  - 连续失败达 `COMPRESS_CIRCUIT_LIMIT`（3）即开闸；
  - 冷却期 `COMPRESS_CIRCUIT_COOLDOWN`（5 分钟）内一律跳过 LLM 压缩——历史不回滚、不
    阻塞回复；
  - 冷却结束放行**一次**（半开），由这次结果决定复位或重开。冷却期内的失败**不刷新**
    开闸时刻（否则"每次失败都重置冷却"导致永不重试）。
- **压缩失败不得阻断主回复**：自动压缩是优化项，历史已回滚，因此它失败最坏的代价是
  "本轮用更长的上下文跑"。`inputs.rs` 里 `auto_compress_process` 的 `Err` 从"让整轮
  `TurnExit::Failed`"改为"告警 + 继续"——这是"反复压缩失败"体验糟糕的**根因之一**，
  现在压缩失败只是日志里一行，用户的消息照常得到回复。

**会话 ID 默认短 GUID**（任务 25，与本次同源）：

`vdfs_provider.rs` 新建会话用 `uuid::Uuid::new_v4()`（36 字符带连字符），而项目已有两处
8 位短 ID 约定（`turn::short_id` / `vdfs_service::entry::auto_id`）。会话 ID 是 VDFS 目录名
（`.symbio/session/<id>`），也是用户可见地址，应一致。改为 `crate::symbio_core::turn::short_id()`
+ 目录碰撞重试（重试仍撞则错误外抛——目录冲突属真异常，不该静默吞）。

### S20.7 —— 三个「静默丢弃」断点（本次）

S20.3~S20.6 修的是**服务端**的状态收敛。本次处理的是同一类病灶在**另外三处**的复发，
三个症状看起来无关（顺序倒挂 / 重试入口不出现 / 一直转圈），实际都在同一条链上：
**丢一次就永久错，且没有任何机制会纠正它**。

| # | 症状 | 断点 |
|---|---|---|
| ① | 每次压缩后消息顺序倒挂（`mtmae8j2wxam4dhrei`） | `assign_seq` 从 `base` **向上**填号 ⇒ 改写保留区既有序号 |
| ② | 中止后看不到「重试」入口（重开才有） | 前端状态词表漏了 `aborted` ⇒ 整条状态被静默丢弃 ⇒ 兜底成 `completed` |
| ③ | 工具已完成，前端一直「运行中」 | 终态补丁丢失（订阅表为空 / 消费循环丢帧）⇒ 前端只做增量收敛，无人纠正 |

| 层 | 改动 |
|---|---|
| `symbio_core/.../chat_message.rs` | `assign_seq` 重写：无号项从**相邻的既有序号向外让位**（开头段整体往下、末尾段往上、夹缝用间隙）；既有序号一个都不改 |
| `session/compression.rs` | 新增 `snapshot_slot_seq`（`保留区首条 seq − 1`）；快照带上槽位序号 |
| `session/chat_loop/compress.rs`、`session/plugin.rs` | 整表重写后广播 `emit_transcript_rewritten`（逐条 `deleted` + 首条 `created`）——此前压缩完全不发 VDFS 变更 |
| 前端 `schemas/vdfs.ts` | 补 `VDFS_STATUS_ABORTED`（`MessageStatus::as_str()` 早已产出这个词） |
| 前端 `services/vdfsTranscriptSync.ts` | `messageStatusOf` 纳入 `aborted`；未知状态词改 `logger.warn` 留痕，不再静默丢弃 |
| 前端 `stores/sessions.ts` | 新增 `reconcileTranscript`；`applySessionNode` 在 `!nowWorking && hasUnsettledNodes(id)` 处触发 |

**①为什么开头段必须"整体一次算好"**：逐个"向上找空位"会先占用首号本身、把既有序号顶成
下一个号——这正是"seq 随压缩改变"的直接来源。由"首号是最小号"可知
`首号 − k … 首号 − 1` 全部落在既有序号之外，故这一段天然不撞号。
**真正的修法在调用方**：给快照显式指定槽位序号，使新列表**本来就单调**，
`assign_seq` 于是退化为"只填缺号"——顺序与稳定同时成立。

**②为什么"漏一个状态词"是终态级事故**：`messageStatusOf` 返回 `undefined` 时
`messageFromNode` 不写 `status`，节点以"无状态"落进 store，而
`registry/messageTypes::messageStatusOf` 的缺省兜底是 `completed`。
于是"用户按了停止"被渲染成"正常结束"，重试入口（挂在 `aborted` 终态上）随之消失。
重新打开会话走叶子 JSON 直读，状态原样保留——所以症状是"重开才有"。

**③为什么自愈用回读而不是就地定稿**：就地标 `completed` 是猜。服务端可能定稿成
`aborted`（用户中止）或 `failed`，猜错即 ② 的重演。回读拿到的是权威终态。
**触发判据不是启发式**：节点补丁恒先于会话状态下发（正常收尾「清在途 → 复位
`is_working` → `emit_session_state`」，中止收尾「`converge_inflight` 广播节点终态
→ `emit_session_state(aborted)`」——见 `handle_abort` 的顺序注释），
故会话报"不忙"时服务端每个节点都已是终态且已落库。触发条件只在**有候选**时成立，
正常运行路径零 IPC。

> **S25 起这条判据从「跨通道的调用顺序」升级为「结构性保证」**：会话运行态帧与
> 消息帧共用同一个 `seq` 计数器、走同一条 `mpsc`，因此「读到 `status != working`
> 的帧」⇒「所有 `seq` 更小的帧都已在它之前应用」不再是发送端的调用顺序，
> 而是**传输层给出的保证**。前端那条宽限复查（300ms）与整份回读随之**整体删除**
> （§11）——不是被更强的网替代，而是它要补的那个缺口不再存在。
> 上面这段「节点补丁恒先于会话状态下发」的**服务端**顺序要求仍然有效：
> 它保证的是"会话报不忙时节点已终态"，与通道无关。

### S20.8 —— 压缩失败**不得裁剪历史**：失败必须是可见、可重试、可持久化的状态（本次）

S20.6 把压缩失败做成了"可诊断 + 可熔断"，但漏了一处：**输入超限预判分支仍然会本地
机械截断历史，并且 `return Ok(Some(..))` 回报成功**。于是压缩节点显示
「已压缩上下文（N → M 条）」`Completed`——没有原因、没有重试入口，历史凭空变短。
（决策记录见 `docs/DECISIONS.md` ADR-018。）

**触发时机恰恰是"压缩反复失败的终点"**：LLM 请求失败 → 回滚（历史完整）→ 下一轮再
失败 → 熔断（跳过自动压缩）→ 上下文继续增长 → 越过模型输入上限 → **本地截断**。
即：**压缩失败的最终代价由历史买单**。

| # | 症状 | 断点 |
|---|---|---|
| ① | 压缩放不下就把历史砍短，且节点报"已完成" | 预判分支走 `emergency_tail_compression` 后 `Ok(Some(..))` |
| ② | 用户看到历史变短却没有任何原因 / 重试入口 | 失败被伪装成成功，且压缩节点**没有**重试粒度 |
| ③ | 重试无处可发 | `messageRetryTargetOf` 只有 `retry` / `retry_turn` 两种粒度，压缩节点（**根级**）落兜底分支会被后端 `process_retry_turn` 以 NotFound 拒绝 |

| 层 | 改动 |
|---|---|
| `session/chat_loop/compress.rs` | 预判命中时**仍跳过 doomed 请求**（只保留这一收益），改为 `Err(CompressionFailure::InputOverLimit { pending, limit })`；`NoPayoff` 变体删除 |
| `session/compression.rs` | 删除 `emergency_tail_compression`（连同 4 条单测）——"强制裁剪"的唯一实现 |
| `schemas/session/chat_message.rs`、`session/resume.rs` | 新增 `ResumeAction::RetryCompaction` |
| `session/chat_loop/compress.rs` | 新增 `retry_compaction`；`auto_compress_process` 的 `force = true` 同时绕过熔断 |
| 前端 `registry/messageTypes.ts` | 新增 `canRetryCompaction` + 第三种粒度 `retry_compaction`（必须在工具分支**之前**分派） |
| 前端 `components/message/CompressionNode.vue` | 失败形态走 `MessageErrorBox`（故障红 + 重试），不复用成功态的弱化样式 |

**为什么"重试"必须给，即便当场重试仍会失败**：`input_over_limit` 重试当然还是超限——
但用户**换一个上下文更大的模型**后，同一次压缩就能成功。按 `failure_kind` 分档
"聪明地"不给入口，会掐掉这唯一的出路。原因里含"待压缩量 vs 上限"两侧数字，足够
用户判断要换多大的模型；判断权归用户。

**为什么预判（跳过 doomed 请求）保留、裁剪（丢历史）删除**：两者的收益与代价完全
不对称。跳过的收益是"不再每轮白等数分钟"——零代价、纯收益；裁剪的收益是"水位立刻
回落到可工作区间"——代价是**不可逆地丢掉用户的历史**，而且失败还被伪装成成功。
"压缩放不下"只说明需要更强的模型或更小的输入，不构成"可以静默丢历史"的授权。

**代价（明说）**：历史真的超出模型输入上限且压缩始终失败时，本轮请求会撞 provider 的
context-length 错误而失败（带原因 + 重试入口），而不是"带着残缺历史继续跑"。
这是刻意的取舍——可见的失败优于静默的数据损失（ADR-018）。

### S21 —— 工具调用请求显式化（**本次不做，留待需要时**）
把 ToolCall 的参数从 `content` 提升为一个真子节点（`type = tool_request`）。
**现在不做**，因为它要求 `plugins/model/message_builder` 的请求扁平化同步改造，
而收益只是"地址更纯"——§2.2 已论证：请求就是 ToolCall 的**内容**，
两个地址会让同一份参数存两处。**记录在案，避免下一个人重新论证一遍。**

### S24 —— 消息帧收成「一条消息」（本次）

S23 把消息实时面从「VDFS 变更」收成一条转写流，但帧仍带一层**显式操作枚举**
（`NodeOp`：`upsert` / `append` / `remove` / `reset` / `warn`）。本次把这一层删掉：
**帧就是一条 `ChatMessage`**（`NodeEvent = { session_id, seq, message }`），语义全在字段上——
`delta` 追加 / `content` 整条替换 / `status = removed` 就地移除 / 其余字段合并。

| 帧里有什么 | 接收端动作 | 原 `NodeOp` |
|---|---|---|
| `delta` | 追加到该节点正文尾部 | `Append` |
| `content` | 整条替换该节点正文（幂等） | `Upsert`（正文部分） |
| `status = removed` | 就地移除该节点 | `Remove` |
| `status`（其余） / `error` | 状态迁移 | `Upsert`（状态部分） |
| 身份字段 / `meta` / `seq` / `timestamp` | 有则合并 | `Upsert`（身份部分） |

**为什么能删**：操作枚举只是同一条消息的**字段子集**的另一种编码。收掉之后，
「消息现在是什么样」只有一个来源——消费端不必把两套结构对齐，也不必从帧的形状
推断「该拼接还是该替换」（`delta` / `content` **互斥**，同帧携带即协议违例：后端
写入点 `Transcript::apply` 与前端落地 `applyTranscriptMessage` 用同一条判据报错丢弃）。

**`reset` / `warn` 的去处**：
- `reset`（清空重读）**不存在于协议**——消费端要重读只有一条路：自己发现序号缺口
  （或收到后端的 resync 标记）。
- `warn`（会话级告警）**不是消息**：它下沉为 `TranscriptWriter::warn(Option<String>)`
  的独立通道，落在会话节点（VDFS watch 域）上，不再占一帧转写流。

**一条必须记住的边界**：状态帧（`state_frame`）会**剥掉正文**，因此它只适用于
「正文已由 `delta` 逐帧上线过」的节点；正文**尚未上线**的节点（一次性节点 / 结果
正文首次到达，如压缩节点的终态）必须用**完整消息帧**（`message_frame`），否则消费端
只拿到状态、永远停在占位文案上——S20.4 的压缩结果文本踩过这个坑（`CompressionEmitter::finish`）。

---

## 7. 验收：用户体验等价清单

迁移的硬要求是**体验不比现在差**。逐项对照：

| # | 现有体验 | 迁移后 | 依据 |
|---|---|---|---|
| 1 | 发送后立即显示"处理中…" | 不变（本地乐观置位保留，随后被权威节点状态覆盖） | `useChatConnection.send` |
| 2 | 流式逐字输出 | 不变（`appended` + `delta` 就地拼接，零回读） | §3.2 |
| 3 | 思考 / 工具调用单行折叠 + 状态标签 | **更强**：标签覆盖 `运行中 / 待确认 / 失败`（此前 `待确认` 的 warn 样式是死代码），运行中另带三点脉动 + 已运行时长 | §5.3.1 |
| 4 | 工具审批：卡片角标 + 表单 | 不变（`waiting_user_action` 节点状态，本就是 VDFS） | §5.2 |
| 5 | 失败：根级 Turn ⚠ + 重试 | 不变（失败节点状态） | §5.2 |
| 6 | 失败但无节点（transport 级） | 会话节点 `failed` + `attributes.error` → 同一条错误条 | §3.3 |
| 7 | 中止：状态回落 + "已中止" | 会话节点回落 `active`，`outcome = aborted` | §3.3 |
| 8 | 结束提示音（三音色） | 不变（状态迁移 + `outcome`） | §5.2 |
| 9 | 切会话来回不丢状态 | **更强**：状态是节点属性，不依赖回放缓冲 | §4 |
| 10 | 会话列表状态点 | 不变（节点 `status`） | §5.2 |
| 11 | 重连后状态收敛 | 不变（`list` 快照 + 只升不降规则） | §4.2 |
| 12 | 中止后不再有节点转圈 | ToolCall / reason / Turn 全部落终态；Turn 落 `Failed` 以给出重试入口 | §6 S20.3 |
| 13 | 切回「正在跑」的会话 | 正在跑的那一轮**仍在**（叶子读含在途 + 前端水合不丢弃在途） | §6 S20.3 |
| 14 | 长会话触发压缩时 | 消息流里出现「上下文压缩」节点：运行中脉动 + 已用秒数，完成后显示"已压缩上下文（X → Y 条）"（此前**完全无提示**，表现为卡死） | §6 S20.4 |
| 15 | 压缩请求的内容 | 历史以**对话形态**下发（不是 JSON 转储），末尾一条压缩指令 | §6 S20.5 |
| 16 | 压缩失败 | 节点落到 `Failed`/`Completed(aborted)`，且 `error` + `meta.failure_kind` 写明**原因**（Llm / InvalidSnapshot / InputOverLimit）；不再是一句无差别的"压缩未完成"且无 `meta` | §6 S20.6 |
| 17 | 压缩连续失败 | 达阈值后熔断 5 分钟冷却，期间**跳过 LLM 压缩**但仍正常回复（不白等、不阻断）；冷却结束半开重试一次 | §6 S20.6 |
| 18 | 压缩失败后的历史 | **一条不动**（四种失败出口全部还原为调用前列表）；节点上是错误条 + 原因，不再是"已压缩上下文（N → M 条）"的成功态 | §6 S20.8 |
| 19 | 压缩失败后重试 | 失败节点上给「重试」入口（resume `retry_compaction`）：删失败节点 → 重新压缩；绕过熔断（用户主动意愿不受冷却约束） | §6 S20.8 |

---

## 8. 不变量

1. **一个节点一个地址一份状态**：状态变更只发生在节点上，不在通道上。
2. **状态变更携带全量节点视图**：`updated` 必带 `node`；消费者**不得**为状态回读。
3. **热路径窄**：`appended` 只带 `delta`。
4. **前端不按事件类型分派**：只有 `routeOf(地址)`；没有 `switch (event.type)`。
5. **派生是纯函数**：等待审批、活动文字、可重试性都由节点表算出，不记录"历史事件"。
6. **补丁与变更同生共死**：任何送达前端的会话状态变化都必须经同一个出口
   （后端 `emit_session_state`），漏发即 UI 永久停在旧状态且无人纠正。
7. **`completed` 不再坍缩为 `active`**：消息状态原样透传，消费端不做还原。
8. **会话状态词只有三个**：`working` / `active` / `failed`——不为会话造 `pending` /
   `completed`（它没有"未开始"与"已结束"）。
9. **失败是状态不是标志**：不再有 `last_failed` 布尔。
10. **实时面一条流、一个 `seq` 空间**（S25 起）：消息（`transcript_event`）与会话运行态
    （`transcript_session`）都走 `session/stream` 转写流，且**从同一个计数器取号**
    （`Transcript::emit` / `Transcript::emit_session_state`，`seq` 的唯一分配点）。
    因此「会话报不忙 ⇒ 本轮消息终态帧都已落地」是**结构性保证**，不是调度巧合
    ——这正是 S20.7 ③ 那条自愈网（宽限复查 + 整份回读）得以删除的理由。
    **VDFS 变更通道只承载资源信号**（创建 / 删除 / 改名 / 标题 / metadata），
    且**不携带会话节点快照**：一条无序通道上的快照会与有序通道上的状态竞争
    （一次迟到的自动命名就能把运行态回退成它自己那一刻的旧值）。
    判断「本轮 / 子会话结束」看会话节点的 `status`（不再有 `Status idle` 帧可等），
    **不是**根 Turn 的终态（一轮里它会多次定格）。旧事件频道（`kind = "session"`）
    已整体废除——**而不是**"前端不订、后端还发"。前端
    （`services/transcriptStream.ts`）、CLI（`cli/src/client.rs`）与子智能体转播
    （`agent/host/subagent.rs`）三处消费者都订阅这条流。
11. **ToolCall 的 `streaming` 覆盖执行窗口**：`finalize_assistant_turn` 不得提前定格；
    每个 ToolCall 都必须以终态收场（未执行者收口为 `Completed` +
    `meta.failure_kind = "not_executed"`），不得有节点停在 `Streaming`（§5.3.1）。
12. **运行时长是节点属性**：`meta.started_at` 由执行方写入；前端不自造锚点，
    锚点缺失即不显示时长。
13. **中止自行收口，不等 chat_loop**：`handle_abort` 一旦复位 `is_working`，消费循环
    就可能因 `!is_working` 提前 `break` 而跳过 `persist_failure`。因此中止路径自己调
    `converge_inflight`，且必须**幂等**（`handle_abort` 与消费循环的中止出口都会跑）。
14. **同一份数据的两个地址给出同一份内容**：会话叶子 `read` 与转写列表都叠加在途缓冲
    （`overlay_live`）；在途副本**继承**存储分配的 `seq`——顺序锚点只由存储分配，
    否则同一条消息会以「有 seq / 无 seq」两种形态排到列表的两个位置。
15. **系统动作也是节点**：压缩不挂会话级横幅，而是 `msg_type = compression` 的**消息
    节点**——它有位置（当前时刻）与状态（进行中 / 已完成），可回溯；横幅没有位置概念，
    滚动即消失。但它**不是对话内容**，`flatten_chat_messages` 必须把它剔除出请求包
    （§6 S20.4）。
16. **给模型的输入按模型的视角投影**：压缩请求把历史**原样**交给 provider 的
    `flatten_chat_messages`，而不是 `serde_json` 序列化存储结构——后者会为 `id` /
    `parent_id` / `seq` / `meta` 等模型完全不关心的字段原样付费（§6 S20.5）。
17. **失败要留证**：节点的 `Failed`/`Completed(aborted)` 必须带 `error`（人读原因）与
    `meta.failure_kind`（机读码），不得无差别地落"未完成"却留空 `meta`/`error`——否则事后
    无法区分"LLM 挂了"与"摘要结构非法"（§6 S20.6）。
18. **优化项失败不得阻断主路径**：自动压缩是上下文优化、失败时历史已回滚，因此它失败
    最坏的代价是"本轮用更长上下文跑"，绝不能 `TurnExit::Failed` 让用户消息也拿不到回复
    （§6 S20.6）。
19. **连续失败要熔断**：只看当前水位不看历史，会让"注定失败的压缩"每轮重试、每次都白等
    一次完整 LLM 往返。达阈值即开闸冷却，冷却期内跳过而非硬扛（§6 S20.6）。
20. **丢了怎么自愈，两条通道各有承诺**：消息流（`session/stream`）带会话内单调 `seq`
    ——跳号即**已知有损**，恢复路径是整份重读（后端还会主动补发 resync 标记），
    重读源是 VDFS 读面、权威且无需补帧（`services/transcriptStream.ts`，与后端
    `transcript_stream::publish_frame` 的背压策略成对）。会话运行态（`kind = "vdfs"`）
    则**不重放**——订阅表为空时 `ChangeSubscriptions::notify` 直接返回（watch 登记是
    fire-and-forget 的异步动作）。因此消费端不得把"没收到终态"就地把节点猜成
    `completed`——服务端可能定稿成 `aborted` / `failed`，猜错就是"把半截谎报成
    正常结束"。
21. **状态词表前后端逐字同源**：后端 `MessageStatus::as_str()` 产出的每一个词，
    前端 `schemas/vdfs.ts` 都必须登记。漏一个的代价不是"少显示一个标签"，而是
    **整条状态被静默丢弃**（消费端 `messageStatusOf` 的未知分支），节点以"无状态"
    落进 store，被缺省兜底成 `completed`——终态被谎报。故未知状态词一律 `logger.warn`
    留痕，且新增状态词时两侧一起改（§6 S20.7 ②）。
22. **压缩失败不改动历史**：四种失败出口（LLM 失败 / 快照校验失败 / 输入超限 / 用户
    中止）全部把消息列表还原为调用前状态，**一条不丢**。历史是用户的资产，"这次压缩
    放不下"不构成"可以静默丢历史"的授权（§6 S20.8，ADR-018）。
23. **失败不得伪装成成功**：压缩的输入超限预判只允许"跳过注定失败的请求"，**不得**
    顺势本地截断并回报 `Ok`——那会让节点显示"已压缩上下文（N → M 条）"，用户既不知道
    出了事，也没有任何入口可点（§6 S20.8）。
24. **失败态必须给出路**：压缩失败节点必须同时满足三件事——原因可见（节点正文 +
    `meta.failure_kind`）、**可重试**（resume `retry_compaction`）、**可持久化**
    （节点落库，重开仍在）。少了"可重试"，用户唯一的出路（换更大上下文的模型后重试）
    就断了；重试入口**不按 `failure_kind` 分档**，判断权归用户（§6 S20.8）。
25. **有调用必有结果**：ToolCall 的终态补丁与它的**结果子节点**成对出现——
    执行、失败、被拦下、未轮到、中止，五种收场都要有结果（`not_executed_result`）。
    只补父节点（#11 满足）是一个**静默**缺口：卡片有请求、没有响应，而请求视图
    被 `flatten_chat_messages` 的占位兜住 ⇒ 模型看得见、用户看不见。
    显示层另有一条兜底文案（`registry/messageTypes::missingResultNoteOf`，
    按父节点自述的 `meta.failure_kind` 给出），服务于修复前落下的历史数据，
    不是本不变量的替代（§5.3.1）。
26. **组合节点的终态跟随子树**：`turn` / `tool_call` 的终态不得早于它的子树。
    ToolCall 由执行方定格（#11）；根 Turn 由 `finalize_turn_root` 在 `close_turn`
    返回之后定格，**全流程唯一一处**（§5.3.2）。它在此之前于
    `finalize_assistant_turn`（LLM 流结束那一刻）发出，比子树早一整个执行窗口。
    发出的节点与 `persist_messages` 落库的是**同一份**，因此实时帧与存储按同一
    取值收敛，不需要第二套"终态"构造逻辑。
27. **帧语义只由字段给出，不由节点类型反推**：`delta` = 尾部追加，`content` = 整条替换；
    同帧携带两者是**协议违例**（写入点报错丢弃——不发布、不占 `seq`，以免污染缺口检测）。
    追加对正文 / 思考 / **工具参数** / **工具响应**一律成立（后端构造帧只有三处：
    `turn.rs::message_frame`（完整消息）/ `state_frame`（剥正文的状态帧）/ `emit_delta`
    （增量）；工具参数与 Text / Reasoning 同构，工具响应透传子会话的增量帧）。
    消费端按 `type` / `role` 猜"该追加还是该替换"是错误来源：实测曾把
    `role = tool` 的流式响应当成全量重发，正文被**最后一片**覆盖——工具卡片
    有请求、响应是空的。落地动作与字段**一一对应**（`transcriptStream` 的
    `sink.message(...)` 一个入口按字段分派），中间不得再插一层"合并"。
28. **`seq` 与发布逐帧，只有日志可分级别 / 折行**：`Transcript::emit` 对每一帧都分配
    `seq` 并 `publish_frame`，**无例外**；`FrameLogLevel` 只决定这一行进 `INFO`
    还是 `DEBUG`，`DeltaLogCoalescer` 只决定"要不要打这一行、打成什么样"（§10）。
    分级与折行是**可观测性**的取舍，不是协议的取舍——任何让 `seq` 跳号或让某帧
    不发布的"优化"都会破坏消费端的缺口检测（§4.1）。
    **S25 起这条边界的单位从「每条消息帧」扩到「每一帧」**：会话运行态帧同样在
    `Transcript` 里取号并发布（`emit_session_state`），且与消息帧走**同一个扇出**。
    理由不是对称好看，而是不变量 #10 那条推理**要求** `seq` 等于"帧在流里的位置"
    ——运行态帧若另起计数器或不占号，它就不再是一个可用的顺序锚点。
    骨架帧（出现 / 终态 / 等待用户 / 删除）**一律不可折行**：折行要保住的就是它们。
    首帧尤其不能折——`apply` 明确允许"未知 id 的增量帧自给自足"（§8 的帧自给自足），
    故首帧可能恰好是纯增量；它一旦被吞掉，时间线就缺掉一个节点的起点与 `seq`。

---

## 9. 取舍与风险

| 项 | 说明 |
|---|---|
| ~~`kind = "session"` 未整体删除~~ | **已补完（S22）**：进程内消费者（`agent/host/subagent.rs` 的审批透传 / 文本累积、`cli/src/client.rs` 的渲染与完成判定）已全部改订阅 VDFS 变更，常量 `KIND_SESSION` 随之移除 |
| ~~`event_bus/pending/snapshot` 路由保留~~ | **已补完**：回放缓冲的唯一数据源是按 `session_id` 灌入的事件帧，VDFS 变更发布方传 `session_id = None`，缓冲永远为空——路由与缓冲（`PENDING_EVENTS` / `drain_pending`）已一并删除 |
| 会话节点状态无独立版本号 | 依赖 §4.2 的三条假设。加 `rev` 需要跨进程单调时钟，收益不足以抵消脆弱性——宁可把假设写清楚 |
| 会话节点 `content` 为空 | `read(<根>/session/<sid>)` 仍是整份会话 JSON（历史读入口），`updated` 变更带 `content` 会白白重传整份历史。**因此会话节点的 `updated` 只带 `node`，不带 `content`**——`node` 足以表达状态，正文另有 `消息` 列表承载 |
| `attributes.outcome` 是场景字段 | VDFS 只透传（与 `message_count` / `meta_tags` 同一手法），不构成机制新增 |
| 核心日志的分级 + 增量折行 | 默认输出只剩**骨架**（出现 / 终态 / 等待用户 / 删除），过程细节（`Update` 帧与折行统计）要 `--verbose` 才见。代价是"某一帧长什么样"不再直接可见；换来的是时间线重新可读（§10）。需要逐帧细节时按 `seq` 区间去转写流里取，不靠日志 |

---

## 10. 批次 K：核心日志分级与折行（已落地）

### 10.1 问题：时间线被同一件事填满

`Transcript::emit` 的注释原本写着「每帧一行，即时间线本身」。这句话对**状态帧**
（开始 / 增长态 / 终态 / 删除）成立，对**纯流式增量**不成立——增量帧数由模型决定，
不由人决定。

实测（真实模型，一段约 200 字回复）：

| | stderr 总行数 | 帧日志行数 | 其中增量帧 |
|---|---|---|---|
| 折行前 | **622** | 587 | **580** |
| 折行后 | **44** | 9 | 2（区间统计） |

即 93% 的日志是 `[T#N] <id> - Update +1c` 这类行。要在这几百行里找"工具什么时候
开始跑的"需要 `grep`，而时间线本该一眼看完。

### 10.2 改法：只折日志，不碰协议

新增 `DeltaLogCoalescer`（`transcript.rs`）：相邻的、**同 `message.id`** 且
**纯流式增量**（带 `delta`、无 `status`）的帧并入同一个 run，不产出日志行；
换 id、或该帧带了状态迁移 / 全量正文、或显式冲刷（`persisted` / `clear`）时，
把 run 渲染成**一行**。

```text
单帧（与折行前逐字相同）      [T#7] a - Update +3c
多帧（带区间 / 帧数 / 累计字符）[T#4..473] a - Update 470 帧 / +470c
```

状态帧（`Start` / `End` / `Removed` 等）**一律不折**——它们才是时间线上的骨架。

`seq` 分配与 `publish_frame` **一行代码都没动**——折行器只被 `emit` 用于决定日志。
（§10.5 之后，折行统计行落在 `DEBUG`：默认输出里它也不再出现。）

### 10.3 折行后的时间线（同一场景）

```text
[T#1] u… - Start
[T#2] 8d8c16bb streaming Start
[T#3] ed59707c streaming Start        ← 推理节点
[T#4..328] ed59707c - Update 325 帧 / +1536c
[T#329] e8f0b51b streaming Start      ← 正文节点
[T#330..611] e8f0b51b - Update 282 帧 / +479c
[T#612] ed59707c completed End
[T#613] e8f0b51b completed End
[T#614] 8d8c16bb completed End
```

结构一眼可见：两个内容节点各自增长了多少、谁先谁后、`seq` 如何分配。

（下面这段是**折行刚落地**时的形态。§10.5 分级之后，`Update` 统计行移入 `DEBUG`，
默认输出只剩 `Start` / `End` 这些骨架行。）

### 10.4 验证

- `cargo test --lib` 基线 +6；其中 `coalescing_does_not_touch_seq_or_the_graph`
  专门钉住"折行没碰 `seq` 与图"这条边界——它才是这次改动的真正风险所在。
- **CLI 端到端（确定性）**：`mock-chat-split` 场景（单行 809 字节、逐字节写出）
  stdout 与期望文本**逐字节一致**；数百个增量帧折成 1 行
  `[T#a..b] … N 帧 / +Nc`，`seq` 连续无缺口。
- 真实模型冒烟：`EXIT=0`，输出完整。

### 10.5 续：日志分级（骨架 / 细节）

折行把"一次回复几百行"降到几行，但**剩下的行并非同等重要**。一轮工具对话里真正
要读的是**骨架**：什么节点出现了、什么时候结束、什么时候停下来等人；而 `Update`
帧（`pending → streaming` 的迁移、正文替换、仅 `meta` 变更）与折行统计都只是过程量。

于是核心日志分两级（`FrameLogLevel`），判据是**相位**（`frame_log_of`，机械判别）：

| 相位 | 条件 | 级别 | 例 |
|---|---|---|---|
| `Start` | 节点在帧前不在内存图中 | `INFO` | `[T#3] 6004d9e9 streaming Start =12c` |
| `Wait` | 迁到 `waiting_user_action` | `INFO` | `[T#12] bca66001 waiting_user_action Wait` |
| `End` | 迁到任一终态词（`completed` / `failed` / `aborted` / `removed`） | `INFO` | `[T#14] 05a937cd completed End` |
| `Update` | 其余 | `DEBUG` | `[T#15] bca66001 streaming Update` |

折行统计行（`[T#4..583] … N 帧 / +Nc`）同为 `DEBUG`。**折行只对 `Detail` 开放**——
骨架帧一律各自留行，`DeltaLogCoalescer::feed` 的 `foldable` 参数就是这个开关。
首帧尤其不能折（§8 不变量 #28）。测试侧的锚：
`a_skeleton_frame_is_never_folded`、`frame_log_splits_skeleton_from_detail`。

`waiting_user_action` 从"被当作终态"改为**独立的骨架相位**：它既不是结束（用户回完
节点还会继续长），也绝不该沉到 `DEBUG`——**要人做事**的时刻必须默认可见。

实测（CLI，一次「推理 + 正文 + 工具调用 + 收尾」的两轮对话）：

| | stderr 总行数 | 转写帧行数 |
|---|---|---|
| 只要等级（默认 `INFO`） | 47 | **14**（全是骨架） |
| `SYMBIO_LOG=debug` | 69 | 19（+ 5 行细节：`Update` 与折行统计） |

默认输出（`INFO` 的全部转写帧行，逐字）：

```text
[T#1] u1a0c89257fa0 completed Start =18c      ← 用户消息
[T#2] a299b4ea streaming Start                ← Turn
[T#3] 6004d9e9 streaming Start =12c           ← 推理节点
[T#5] bf2f3488 streaming Start =0c            ← 工具调用（参数从空开始，逐帧长到 22c）
[T#8] ce528d47 streaming Start =6c            ← 正文节点
[T#13] 6004d9e9 completed End
[T#14] ce528d47 completed End
[T#16] ee4031f0-… completed Start =28c        ← 工具响应
[T#17] bf2f3488 completed End                 ← 工具调用终态（在结果之后）
[T#18] a299b4ea completed End
[T#19] 47562921 streaming Start               ← 下一轮
[T#20] 10b9741d streaming Start =6c
[T#27] 10b9741d completed End
[T#28] 47562921 completed End
```

骨架一眼看完：**谁出现、谁结束、`seq` 怎么排**。`INFO` 与 `DEBUG` 的差别**只在
stderr 的行数**——`seq` 分配、帧发布、前端实时面一个字节都没变（不变量 #28）。

---

## 11. 批次 E：会话运行态并入转写流（已落地）

### 11.1 问题：一条**推不出来**的结论

S20 把会话运行态搬到了会话节点上，从此状态是幂等的全量视图、可丢可重放——这一半是
对的。但它与消息走的是**两条通道**：运行态走 `kind = "vdfs"`（`event_bus` 的
`mpsc`，容量 4096），消息走 `session/stream`（`transcript_stream` 的 `mpsc`，容量
2048），各有一个泵任务。

于是「会话报不忙」**推不出**「本轮消息节点都已收到终态帧」——两者只是两个独立任务
的调度顺序，没有任何机制保证。失败症状是**静默**的：某个节点永远停在 `streaming`
（前端显示"运行中"，下一次发送还会把它当成"已有在途节点"），而没有任何机制会纠正。

S20.7 ③ 给它挂了一张兜底网（宽限 300ms 复查 + 仍不收敛就整份回读）。网是对的，
但它是**症状治疗**：它把"无声失效"降级为"一次可观测的额外读取"，代价是**每轮
收尾都要挂一个定时器**，且在真的乱序时每轮一次全量转写读取。

### 11.2 改法：把结论变成结构

> 让运行态帧与它那一轮的消息**从同一个计数器取号、走同一条流**。

`seq` 的唯一分配点是 `Transcript`（`self.seq += 1`，`clear()` 也不复位 ⇒ 会话内
跨轮单调）。运行态帧因此必须**在 `Transcript` 里取号**：

```text
Transcript::emit(msg)              → seq = n   → publish_frame
Transcript::emit_session_state(node) → seq = n+1 → publish_session_state
```

两者走**同一个 `fan_out`**（同一个 `subs` 表、同一个 `mpsc`），于是单通道保序 +
`seq` 严格递增直接给出：

> 读到 `status != working` 的这一帧 ⇒ 所有 `seq` 更小的帧（含本轮全部终态帧）
> **都已在其之前被应用**。

这是**传输层的保证**，不是发送端的调用顺序。兜底网随之**整体删除**——不是被更强
的网替代，而是它要补的那个缺口不再存在。

### 11.3 为什么资源变更**留**在 VDFS（不是遗留）

运行态帧需要 `seq`，而 `seq` 只在 `Transcript` 里分配。创建 / 删除 / 改名 / 标题 /
metadata 覆盖这些变更**发生在没有在途轮次的时候**——此刻没有活跃转写可依附，
拿不到号。这是**结构性理由**，不是"还没来得及搬"。

因此分工是：

| 会话叶子上变的东西 | 通道 | 粒度 |
|---|---|---|
| 运行态（`working` / 终态 / 结局 / 警告 / 错误） | 转写流 `transcript_session` | 带**全量节点视图**，与消息共用 `seq` |
| 资源（创建 / 删除 / 改名 / 标题 / metadata） | VDFS `notify_change` | 只报"变了" |

### 11.4 关键的一条：VDFS 侧**不再携带会话节点快照**

`notify_session_state`（带 `node` 的 `updated`）**删除**，三处调用点改走
`notify_change` 的粗粒度信号；`session_change()` 连同它的单测一并删除。

理由是**快照的来源必须有序或幂等**：

- 转写流的运行态帧——有序（同一个 `seq` 空间）；
- `list` / `stat`——幂等回读。

VDFS 变更是一条**独立的无序通道**。在它上面捎带快照，消费端一旦照单应用 `status`，
一次**迟到的改名**就能把运行态**回退**：自动命名发生在轮次中，那一帧的快照说
`working`，而转写流早已报 `finished` ⇒ 角标永远转。

这不是假想：它正是"两条通道都携带同一份状态"的必然结果，只是原先两条通道**都**是
VDFS（同一条有序通道），所以没暴露。把运行态搬到转写流之后，**任何**留在 VDFS 上的
快照都会立刻变成跨通道竞争源。故一并删除，而不是"留着但消费端别读 `status`"
——后者是一个"看起来权威、实际必须忽略"的字段，正是本仓库反复否决的那类陷阱。

### 11.5 改动落点

| 层 | 改动 |
|---|---|
| `symbio_core/transcript_stream.rs` | 新增 `SessionStateEvent` / `SESSION_TYPE` / `session_state_of` / `publish_session_state`；`encode` + `fan_out` 与消息帧共用 |
| `session/transcript.rs` | 新增 `Transcript::emit_session_state(node)`——**从消息帧那个计数器取号** |
| `session/orchestrator/broadcast.rs` | `emit_session_state` 第 2 步由"发 VDFS 变更"改为"发转写流帧"；先取节点视图（会话已删则静默返回） |
| `session/plugin.rs` | 新增 `session_node_of`（节点视图的单一构造点）；**删除** `notify_session_state` |
| `session/{handlers,orchestrator/entry,plugin/vdfs_provider}.rs` | 三处资源变更改走 `notify_change` |
| `session/plugin/nodes.rs` | **删除** `session_change`（`VdfsChange.node` 至此再无生产性生产者） |
| 前端 `services/transcriptStream.ts` | 新增 `transcript_session` 分派 + `advanceSeq`（**两种帧共用**的缺口检测）；`flushPendingFrames(sessionId?)` 支持**按会话冲刷**；**删除** `reconcileTranscript` |
| 前端 `stores/sessions.ts` | `applySessionNode(id, change)` → `applySessionState(id, node)`；**删除** `RECONCILE_GRACE_MS` / `reconcileTimers` / `scheduleReconcileTranscript` |
| 前端 `stores/sessionNodeSync.ts` | 只留 `deleted` / `created` / `renamed` / `updated` 的**粗粒度**收敛（后三者统一为防抖重拉） |
| CLI `client.rs` / `main.rs` | `Frame` 改为 `Transcript` / `Session` / `Resync`；删 event_bus 订阅与整套 watch 簿记；消息与运行态**共用一个 `advance_seq`** |
| `agent/host/subagent.rs` | 删 `bus_rx` / `register_subscriber` / 两个 watch 地址；`stream_relay_bridge` 11 参 → 9 参 |

### 11.6 前端的顺序细节：运行态帧**不走合帧窗口**

合帧（批次 C，48ms）是**纯性能**机制：把窗口内的**消息帧**攒成一批，一次 store 提交
+ 一次消息树重建。

运行态帧**不参与合帧**：它到达时先 `flushPendingFrames(sid)` 再交付节点视图。理由
不是"状态帧更急"，而是**顺序**——它与消息共用一个 `seq` 空间，同会话里 `seq` 更小的
消息帧必然已在本地队列中；让运行态帧等满一个窗口，等于人为制造一段「后端已报空闲、
本地转写却还是非终态」的窗口，而那正是上层过去要靠宽限复查去兜的东西。攒批省下的
一次提交，远不值这条结构性保证。

（冲刷是**按会话**的：运行态帧只对**它自己那个会话**给出"本轮已完整"，别的会话的
待落地帧留在队列里。）

### 11.7 验证

- `cargo test --lib`：+4（`session_state_frames_share_the_message_seq_counter`、
  `session_state_frame_does_not_touch_the_message_graph`、
  `session_state_frame_is_an_envelope_that_decodes_back_to_the_node`、
  `the_two_frame_kinds_are_not_confusable`），−1（`session_change` 已删除）；
- 前端 vitest：+7 条运行态帧协议用例（含"到达时先冲刷同会话待落地帧"、
  "两种帧共用一个游标不触发跳号"、"缺节点视图仍推进水位"），−4 条自愈网用例；
- e2e 三处用例按 `type === 'transcript_event'` **过滤**，新帧类型对它们不可见
  ——零破坏；另补一条观察 `transcript_session` 的断言（见 `e2e/cases/`）。

### 11.8 代价与边界（明说）

- **运行态帧只在有活跃轮次时发出**。这是设计的一部分（`seq` 只在这里分配），
  不是缺陷：没有在途轮次时状态本来就不变（`active` 是稳态，`failed` 由上一轮
  收尾时的那一帧给出）。
- 重连后漏掉的运行态帧**不补发**——与消息帧同一条恢复路径（跳号 ⇒ 整份重读），
  且 `list` 快照本就含权威 `status`（`refreshList` 的"只升不降"规则仍在）。
- 快照仍然**不带**会话正文（`content`）：`read(<根>/session/<sid>)` 才是历史读入口。
  §9 的那条取舍原样保留，只是载体从"VDFS `updated` 的 `node`"换成了"转写流的
  `transcript_session`"。
