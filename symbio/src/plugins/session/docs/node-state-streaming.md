# 节点状态流（Node-State Streaming）—— 流模式从「事件序列」改为「节点状态」

> 前置阅读：[vdfs.md](../../../../../docs/design/vdfs.md)（机制规范）、
> [vdfs-session-messages.md](./vdfs-session-messages.md)（转写即列表，S16–S19）、
> [core-loop.md](./core-loop.md)（会话主循环）。
>
> 本文回答一个问题：**会话的"正在发生什么"如何只由节点状态表达**，
> 使前端不再消费任何事件序列，从而**不存在事件顺序问题**。

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
.vdfs/session                                  会话清单（目录）
.vdfs/session/<sid>                            会话节点        ← 运行态的承载者
.vdfs/session/<sid>/消息                       转写列表（目录）
.vdfs/session/<sid>/消息/<mid>                 消息节点（列表项）
.vdfs/session/<sid>/AGENTS.md                  会话记忆（文件）
.vdfs/session/<sid>/子会话[/<sub>]             子会话清单 / 子会话节点
.vdfs/session/<sid>/工作目录[/<rel>]           工作目录树
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
| `deleted` | 节点消失 / 转写清空 | — | 移除 |

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
| `fetchPendingSnapshot`（`event_bus/pending/snapshot`） | 同上（后端路由保留为对外 API，前端不再调用） |
| `sessionBusWatcher` 的 `Status/Abort/Error/Connected/Disconnected` 分支 | 会话态改由会话节点承载 ⇒ 整个模块删除 |

> 「事件可以丢、可以重放、可以乱序」这三句在**状态**上成立，在**增量**上不成立。
> 因此 `appended` 的顺序链（`vdfsTranscriptSync::enqueue`）**保留**——它是增量语义的
> 固有属性，不是顺序假设的残留。删的是「全局状态顺序」这一类依赖。

---

## 5. 前端消费模型

### 5.1 按**地址**分派，不按事件类型

```text
kind = "vdfs" 变更
      │
      ▼
  sessionRouteOf(path)      ← 纯函数（`schemas/vdfs.ts`）：地址 → 本域目标，可单测
      │
      ├─ .vdfs/session/<sid>            → sessions.applySessionNode(id, change)
      ├─ .vdfs/session/<sid>/消息       → transcriptSync.clearTranscript(id)
      ├─ .vdfs/session/<sid>/消息/<mid> → transcriptSync.applyMessageNode(id, mid, change)
      └─ 其余地址（清单目录 / 子会话 / 工作目录 / 其他资源） → 不是本域的事，跳过
```

**没有 `switch (event.type)`**——那是"按事件分派"，顺序敏感；
这里是"按地址分派"，每个地址只认自己的状态。

实现上落在**两个订阅作用域**（`eventBus.subscribeVdfsChanged`）：

| 作用域 | 消费者 | 负责的地址 |
|---|---|---|
| `.vdfs/session`，`directChildren` | `stores/sessions.ts` | 会话叶子（清单 + 运行态） |
| `.vdfs/session`，整棵子树 | `services/vdfsTranscriptSync.ts` | 转写列表与列表项 |

两者**互不重叠**：会话叶子只归 store（那是它的状态），转写只归 transcriptSync。
重叠会让同一条消息被写两次——流式文本逐词叠字，是这条设计要防的那类回归。
「一个消费端」指的是**一套分派规则**（`sessionRouteOf` + 作用域判定），
而不是"只能有一个订阅者"。

### 5.2 状态 → 视图，不做推断

| 视图 | 唯一依据 |
|---|---|
| 会话卡片状态点 | 会话节点 `status` |
| 会话卡片"上次失败" | 会话节点 `status == 'failed'` |
| 会话卡片"等待审批" | **派生**：转写中存在 `status == 'waiting_user_action'` 的节点（纯函数，不是事件） |
| 活动文字（"正在思考…"） | **派生**：最近变化的节点的 `status` + `type` + `name` |
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

---

## 6. 迁移阶段

### S20 —— 会话运行态上节点（本次）

| 层 | 改动 |
|---|---|
| `session/plugin/nodes.rs` | `message_status` 不再坍缩 `completed`；新增 `SessionRuntime`（含 `from_state` 单一投影入口）+ `session_node(s, runtime)`（运行态投影到 `status` / `attributes.outcome` / `attributes.error`）+ `session_change` |
| `session/active.rs` | `ActiveSessionStateInner` 增加 `last_outcome: Option<String>`、`last_error: Option<String>` |
| `session/orchestrator/broadcast.rs` | `broadcast_status(status)` → `emit_session_state(SessionStateChange)`：写运行态 → 发**带 `node` 载荷**的会话节点 `updated`；不再为前端广播 `Status` 帧（帧仍发，给进程内消费者） |
| `session/orchestrator/{consume,entry,orchestrator}.rs` | 全部改调 `emit_session_state` |
| `session/plugin.rs` | `notify_change` 旁增 `notify_session_state`（带节点视图） |
| `session/plugin/vdfs_provider.rs` | `is_working(id)` → `session_runtime(id)`（`list` / `stat` / 变更三处同源） |
| 前端 `schemas/vdfs.ts` | 补状态词常量；`parseTranscriptPath` → `sessionRouteOf`（含会话叶子）；新增 `sessionRuntimeOf` / `chimeKindOfOutcome` |
| 前端 `stores/sessions.ts` | `syncSessionNode`（回读）→ `applySessionNode`（**零回读**）：状态 / 结局 / 标题 / 错误就地收敛，状态迁移触发提示音；`last_failed` 布尔删除 |
| 前端 `services/vdfsTranscriptSync.ts` | 分派改 `sessionRouteOf`；`messageStatusOf` 不再把 `active` 猜回 `completed`（仅作旧数据别名） |
| 前端 `components/ModelChatPanel.vue` | 错误条改由**节点表派生**（有失败节点则隐藏），不再在事件到达时判定 |
| 前端 `services/sessionBusWatcher.ts` | **删除**（连同 `MainLayout.vue` 的接线） |
| 前端 `services/eventBus.ts` | 删除 `replayBuffer` / `fetchPendingSnapshot`（失去唯一调用方） |

### S21 —— 工具调用请求显式化（**本次不做，留待需要时**）

把 ToolCall 的参数从 `content` 提升为一个真子节点（`type = tool_request`）。
**现在不做**，因为它要求 `plugins/model/message_builder` 的请求扁平化同步改造，
而收益只是"地址更纯"——§2.2 已论证：请求就是 ToolCall 的**内容**，
两个地址会让同一份参数存两处。**记录在案，避免下一个人重新论证一遍。**

---

## 7. 验收：用户体验等价清单

迁移的硬要求是**体验不比现在差**。逐项对照：

| # | 现有体验 | 迁移后 | 依据 |
|---|---|---|---|
| 1 | 发送后立即显示"处理中…" | 不变（本地乐观置位保留，随后被权威节点状态覆盖） | `useChatConnection.send` |
| 2 | 流式逐字输出 | 不变（`appended` + `delta` 就地拼接，零回读） | §3.2 |
| 3 | 思考 / 工具调用单行折叠 + 状态标签 | 不变（`MessageNode` 状态类，状态词不改） | §2.3 |
| 4 | 工具审批：卡片角标 + 表单 | 不变（`waiting_user_action` 节点状态，本就是 VDFS） | §5.2 |
| 5 | 失败：根级 Turn ⚠ + 重试 | 不变（失败节点状态） | §5.2 |
| 6 | 失败但无节点（transport 级） | 会话节点 `failed` + `attributes.error` → 同一条错误条 | §3.3 |
| 7 | 中止：状态回落 + "已中止" | 会话节点回落 `active`，`outcome = aborted` | §3.3 |
| 8 | 结束提示音（三音色） | 不变（状态迁移 + `outcome`） | §5.2 |
| 9 | 切会话来回不丢状态 | **更强**：状态是节点属性，不依赖回放缓冲 | §4 |
| 10 | 会话列表状态点 | 不变（节点 `status`） | §5.2 |
| 11 | 重连后状态收敛 | 不变（`list` 快照 + 只升不降规则） | §4.2 |

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
10. **事件通道仍在，但不承载显示**：`kind = "session"` 的帧继续发布给**进程内**
    消费者（`agent/host/subagent.rs` 需要 `Update` 与 `Status idle` 判断子会话结束），
    前端不再订阅它。

---

## 9. 取舍与风险

| 项 | 说明 |
|---|---|
| `kind = "session"` 未整体删除 | subagent 宿主在**进程内**依赖 `Update`（子会话审批透传 + 文本累积）与 `Status idle`（结束判据）。把它改造成订阅 VDFS 变更是一次独立的、可验证的收敛，不塞进本次 |
| `event_bus/pending/snapshot` 路由保留 | 前端不再调用，但它是网关对外 API 的一部分，删除属另一件事 |
| 会话节点状态无独立版本号 | 依赖 §4.2 的三条假设。加 `rev` 需要跨进程单调时钟，收益不足以抵消脆弱性——宁可把假设写清楚 |
| 会话节点 `content` 为空 | `read(.vdfs/session/<sid>)` 仍是整份会话 JSON（历史读入口），`updated` 变更带 `content` 会白白重传整份历史。**因此会话节点的 `updated` 只带 `node`，不带 `content`**——`node` 足以表达状态，正文另有 `消息` 列表承载 |
| `attributes.outcome` 是场景字段 | VDFS 只透传（与 `message_count` / `meta_tags` 同一手法），不构成机制新增 |
