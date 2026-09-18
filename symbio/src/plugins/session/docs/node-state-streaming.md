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
| 被 PreToolUse 拦下 / 用户中止 / 交互中断 | `process_tool_calls_async` 末尾统一收口为 `Completed` + `meta.failure_kind = "not_executed"` |

**"本批每一个 ToolCall 都必须以终态收场"** 由 `process_tool_calls_async` 负责保证：
调用前广播 `Streaming`，调用后广播终态，未执行的批尾在函数末尾一次性收口。
漏掉任何一条，前端就会有一个**永远转下去的「运行中」**（比"没有迹象"更糟：
它把"卡住"伪装成"在跑"）。

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
| `session/plugin.rs` | `emit_message_deleted`（逐条）**删除**，改为 `emit_transcript_truncated`（一条）；逐节点删除的唯一来源是 `resume` 的 `StreamEvent::Delete`（消费循环直接转译） |
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
2. **中止后节点停在「运行中」**：非流式工具执行（`PluginPayload::Data` 分支）是一次
   `await`，不轮询通道也不看 `abort_flag`；`handle_abort` 的 3s 兜底一旦触发就会强制
   `is_working = false`，消费循环下一帧因 `!is_working` **直接 `break`，跳过
   `persist_failure`** ⇒ 工具节点永远停在 `Streaming`。这正是「终止后还显示运行中」。

| 层 | 改动 |
|---|---|
| `session/orchestrator/failure.rs` | 新增 `is_inflight`（在途集合的**唯一**定义）与 `converge_inflight`：扫**存储 + 在途缓冲**两个数据源，非终态定稿 `Completed` 并广播补丁，**幂等** |
| `session/orchestrator/consume.rs` | 循环出口记 `exit_state`：中止出口发 `aborted`（此前一律 `completed`，与 `handle_abort` 抢同一个字段，收敛靠 3s 轮询的时序侥幸） |
| `session/orchestrator/consume.rs` | `handle_abort` 复位 `is_working` 后**自己**调 `converge_inflight`——中止路径自己的收口责任，不等 chat_loop |
| `session/tool_executor.rs` | 新增 `wait_tool_abort`：`route_fut` 与它 `select!`，让**非流式**工具也能被中止（abort 帧 / 取消令牌 / 已置位三条来源） |
| `session/resume.rs` | 重跑工具时中止：父 ToolCall 已落库且置 `Streaming`，返回前用 `not_executed_patch(.., "aborted")` 就地定稿并广播 |
| `session/plugin/nodes.rs` | `session_content` 叠加 `live`（与 `transcript_of` 同源的 `overlay_live`）——叶子与转写列表必须是同一份消息集合 |
| `session/plugin/vdfs_provider.rs` | 叶子 `read`（含子会话）取 `live_messages_of`；子会话叠加**它自己**的在途缓冲，不是父会话的 |
| 前端 `stores/sessions.ts` | `hydrateFromHistory` 改**合并**语义：快照权威；快照里没有的本地节点**仅当仍在飞行中**才保留 |

**为什么在途集合不含 `WaitingUserAction`**：它不是「正在跑」，是「等用户回答」——
中止时抹掉它等于把审批入口删了；它也不会让前端显示"运行中"（前端对它有独立的
「待确认」标签）。反之漏掉 `Pending` / `Streaming` 中的任何一个，表现就是一个
永远转下去的「运行中」，而它**不报错、只会一直转**——因此这个集合由测试直接锁定
（`orchestrator.test.rs::inflight_set_covers_..`），不靠读代码。

**为什么 `converge_inflight` 要扫两个数据源**：存储侧覆盖 `resume` 重跑工具时**已落库**
的父 ToolCall（它从不出现在在途缓冲里）；在途侧覆盖尚未落库的流式节点。只扫一个，
另一个场景的中止就会漏收。

**为什么 `wait_tool_abort` 忽略通道关闭**：对端消失（chat_loop 已返回）不是中止信号。
把它当中止会让**每一次正常收尾**都变成"用户中止"，于是正常完成的会话被报成
`aborted`、提示音选错音色。这条曾写错并被回退测试抓到
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

**为什么能在静音窗口里发出去**：`emit_message_patch` → `broadcast_frame` 走
`state.inner.frontends` 与 VDFS 变更订阅，**不经过**被静音的那条 turn channel。
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

---

## 9. 取舍与风险

| 项 | 说明 |
|---|---|
| `kind = "session"` 未整体删除 | subagent 宿主在**进程内**依赖 `Update`（子会话审批透传 + 文本累积）与 `Status idle`（结束判据）。把它改造成订阅 VDFS 变更是一次独立的、可验证的收敛，不塞进本次 |
| `event_bus/pending/snapshot` 路由保留 | 前端不再调用，但它是网关对外 API 的一部分，删除属另一件事 |
| 会话节点状态无独立版本号 | 依赖 §4.2 的三条假设。加 `rev` 需要跨进程单调时钟，收益不足以抵消脆弱性——宁可把假设写清楚 |
| 会话节点 `content` 为空 | `read(.vdfs/session/<sid>)` 仍是整份会话 JSON（历史读入口），`updated` 变更带 `content` 会白白重传整份历史。**因此会话节点的 `updated` 只带 `node`，不带 `content`**——`node` 足以表达状态，正文另有 `消息` 列表承载 |
| `attributes.outcome` 是场景字段 | VDFS 只透传（与 `message_count` / `meta_tags` 同一手法），不构成机制新增 |
