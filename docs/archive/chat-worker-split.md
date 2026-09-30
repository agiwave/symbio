<!-- doc-link-allow D-006: 本文是实施前设计，正文引用的 `plugins/chat/*`、`schemas/chat/*` 等路径尚未创建；路径以「拟新增」标注 -->
# 会话响应性改造：编排（worker）× 对话面（chat）

> **【归档说明】** 本文档是**对话面拆分的实施前设计稿（过程产物）**。它描述的形状是
> `chat` 插件 + `chat/reply` 子模块，该形状**从未实现**——方案经
> [plan/06–09](../plan/README.md) 收敛为 `triage`（判决）× `reply`（措辞）两个能力插件，
> **已全部落地**（批次 S0–S6，见 [09 §5](../plan/09-对话面插件拆分实施方案.md)）。
> 已于 2026-09-29 移入 `docs/archive/`；其中的 ADR 草案已迁为
> [ADR-041](../decisions/session.md#adr-041-会话的响应性由编排层调度--无副作用对话服务承担)
> 与 [ADR-042](../decisions/core.md#adr-042-跨插件调用一律经容器-route--路径常量不持有对方类型)。
> 目标架构与迁移计划见 [plan/README.md](../plan/README.md)，「现在是什么」见
> [CURRENT.md](../CURRENT.md)。本文不再更新。
>
> 前置阅读：[session/docs/core-loop.md](../../symbio/src/plugins/session/docs/core-loop.md)（主循环）、
> [session/docs/node-state-streaming.md](../../symbio/src/plugins/session/docs/node-state-streaming.md)（实时面）、
> [session/transcript/inbox.rs](../../symbio/src/plugins/session/transcript/inbox.rs)（输入入口）、
> [model/message_builder.rs](../../symbio/src/plugins/model/message_builder.rs)（请求视图投影）。

---

## 0. 目标

1. **能直接回答的，直接回答。** 不需要查、不需要做就能说清的，不要启动一整轮工具循环。
2. **用户的补充被综合进正在做的工作，整体处理。** 中途不断补充时，worker 把它们**合并**看待，
   而不是一条补充一轮。
3. **worker 中途也会说话。** 进度汇报是一等公民，不是"最后给一个答复"。

三条指向同一个机制改造：**把"说人话"从模型的自发行为，变成编排层的显式调度。**

---

## 1. 现状与缺口

| 环节 | 现状 | 三条目标下的缺口 |
|---|---|---|
| 输入 | 收件箱 FIFO，**一条消息 = 一轮**，忙则排队不抢占（ADR-026） | 目标 2：补充会各占一轮，不是"整体处理" |
| 决策 | 每条消息都无条件进入完整工具循环 | 目标 1：简单问题也走完整循环，慢且贵 |
| 输出 | 模型在一次 LLM 调用里同时承担「干活」与「说话」，说与不说由模型自发决定 | 目标 3：长任务中途可以一句话都没有 |

**已有能力（复用，不重造）**：运行现状已经是可查询的节点树（会话节点 `status` / `attributes.outcome`
/ `attributes.error`，消息节点的 `type` / `status` / `meta.started_at`）；实时面只有一条通道
（`event_bus` 的 `kind = "vdfs"`，ADR-025）；跨插件调用已有范式（`agent_run` 经
`parent.route(ctx{PATH})` 调 `session/chat/send`）。

---

## 2. 目标架构

### 2.1 两个角色，一条边

```text
┌──────────────────────── worker 容器（composite，现状） ────────────────────────┐
│                                                                              │
│   ┌──────────────────────────┐            ┌──────────────────────────────┐   │
│   │  session（编排 + 工作）    │ route(chat/reply) │  chat（对话面 · 新增）    │   │
│   │  · 唯一编排者             │ ─────────▶ │  无工具 · 无副作用 · 无状态    │   │
│   │  · 唯一转写写入者          │ ◀───────── │  判决 + 措辞                  │   │
│   │  · 工具循环 · 上下文 · 存储 │  返回判决/文本 │                              │   │
│   └──────────────────────────┘            └──────────────────────────────┘   │
│              ▲                                                                │
└──────────────┼────────────────────────────────────────────────────────────────┘
               │ 唯一输入入口（不变）：write(<根>/session/<sid>/inbox/<iid>)
           用户 / 前端 / CLI / telegram
```

**只有一条调用边，且单向。** 因为 chat **无副作用**（不写任何地址、不派活、不起后台任务），
它不需要回调 session ⇒ 不需要反向边 ⇒ 转写写入者仍是唯一的。

### 2.2 插件与地址

| 插件 | 角色 | 挂载点 | 自有路由 | LLM 可见工具 | 配置文件 |
|---|---|---|---|---|---|
| `session` | 编排 + 工作 | `<根>/session` | `session/chat/send` · `session/chat/abort`（不动） | 现有工具集（不动） | `<根>/session/PLUGIN.yml` |
| `chat` | 对话面 | `<根>/chat` | `chat/reply`（**新增，唯一的对外动作**） | **无** | `<根>/chat/PLUGIN.yml` |

> **「无工具」是结构保证。** 模型能调用的工具来自 `traverse(TRAVERSE_AVAILABLE_TOOLS)`
> → `CapabilityVisitor::register`。chat 不注册任何 `Capability`，因此模型的工具集里
> **在结构上不可能**出现 chat 的东西。

### 2.3 chat 的两类调用

| intent | 时刻 | 产物 |
|---|---|---|
| `Triage` | 每一批用户输入到达时（本轮消息 / 抽干到的补充） | **判决**：`Answered` 或 `Escalate`，各带一段文本 |
| `Progress` | 工作中且"太久没对用户说话" | `Report`：一段进度文本 |

### 2.4 两条线：分界规则是一条纯函数（呈现上严格分开）

一次会话里同时存在**两条线**：

| | **对话线** | **工作线** |
|---|---|---|
| 谁在说 | 用户 ↔ 助手 | worker ↔ 任务 |
| 内容 | user 消息 + assistant 文本节点 | 推理 / 工具调用 / 工具结果 / Turn 容器 |
| 谁读它 | chat（回答用户）+ 前端「对话」面板 | worker 的请求视图（`build_request_view`）+ 前端「工作」面板 |
| 窗口 | 自己的（`conversation_messages`，默认 20 轮） | 自己的（`context_messages` + 骨架化 / 淡化） |
| 上下文里的角色 | "我们聊过什么" | "我做过什么" |

#### 分界规则：一个纯函数，三处共用

两条线的差别**只体现在"读法"上**，因此它必须是**一个**投影函数，而不是散落三处的过滤条件：

```rust
/// 拟新增：session/context/conversation_view.rs
/// 对话线：只有"人对人说过的话"。纯函数，可重放（ADR-015 的派生要求）。
pub fn conversation_view(messages: &[ChatMessage], limit: usize) -> Vec<ChatMessage>;
```

**分界判据（只认这两个）**：

> 一条消息进对话线 ⇔ `role = user`（含合并后的补充消息） **或**
> （`role = assistant` 且该节点是**文本节点**、不含 `tool_calls`、非 Turn 容器 / 推理 / 工具结果）。

反过来：`role = tool`、含 `tool_calls` 的 assistant 节点、Turn 容器、推理节点
**一律不进对话线**——它们只属于工作线。

这一个函数被**三处**共用（必须是同一份规则，否则会出现"界面看得到、chat 看不到"这类错位）：

| 使用者 | 用途 |
|---|---|
| chat 的 `context_excerpt`（§3.3） | "我们聊过什么" |
| 前端「对话」面板（§2.5） | 人读的记录 |
| 工作线请求视图（`build_request_view` 的补集） | 明确"哪些不进对话线"，与模型视角对齐 |

#### 两种做法：交叉 vs 分离

用户原本的设想是**分离**：对话历史只收 user 消息 + 面向用户的 assistant 文本，
工具 / Turn 全在工作历史里，chat 另有一个 `StatVisitor` 类机制去取 worker 的进展 summary。
两种做法的对照：

| | **交叉（本方案）** | **分离（原设想）** |
|---|---|---|
| 存储 | **一份**（完整往返） | 两份（对话历史 / 工作历史） |
| chat 的上下文 | 对同一数组做**过滤** | 来自另一份存储 |
| 写入者 | 一个（session） | 两个（对话一个、工作一个） |
| 一条 assistant 消息 | 原样一条（正文与 `tool_calls` 同体） | **劈成两半**（正文 → 对话，`tool_calls` → 工作） |
| 请求包怎么来 | 从完整往返**投影**（不会拼不回去） | 从两半 **join** 回来（拼不回去 = 请求非法） |
| 进展怎么给 chat | 节点 → `RunSnapshot` 投影（§3.3） | `StatVisitor` 收集一份 summary |
| chat 上下文干净吗 | **靠投影保证**（见下方硬约束） | 结构上天然干净 |
| 漂移风险 | 无（同一份事实的两个投影） | **有**（`StatVisitor` 是第二份事实） |
| `seq` / 合帧 / 终态收敛 | 单点保证不动 | 两个写入者，单点保证失效 |

**交叉的代价只有一条，但必须硬性守住**：

> **chat 的请求视图里不得出现 `tool_call` / `role = tool` 节点，也不得出现 Turn 容器与推理节点。**
> 工具结果可能含大块文件内容（几万 token 的源码、命令输出）——把它当"对话"喂给 chat，
> 既贵（每次 triage 都付）又错（chat 会以为用户看过这些）。
> 这条约束的**落点必须是投影函数本身**，不能是 chat 的提示词：提示词约束不了一个已经进包的东西。

**分离买到的是"chat 输入天然干净"，但交叉用那条硬约束就买到了同一件东西**；
而分离要付**双写 + join + 第二份事实**三笔账：

- **双写**：一条 assistant 消息落地时要劈成两半、分别写两份存储；写一半失败即状态不一致。
- **join**：请求包必须把两半拼回一条，拼不回去（顺序 / 归属错）就是 400。
- **第二份事实**：`StatVisitor` 的实质问题**不是"多一个接口"，而是"把节点已有的信息复制一份"**。
  运行现状的 owner 是节点（ADR-015 / ADR-025）：worker 的进展、工具状态、已用时长，
  全都已经是节点属性。再收集一次 ⇒ 必然漂移，表现是"chat 说的和界面上显示的不一样"。
  而 chat 要的"进展 summary"本来就**不是原始事实**，是**面向人的措辞**——它应该由 chat
  从投影出的 `RunSnapshot`（§3.3）自己生成，而不是由另一个插件预先嚼好喂进来。

**结论：存储交叉，输入分离，分界单点。**

- **存储一份**：避免 join 与双写；写入者、`seq` 分配、终态收敛的单点保证全部不动。
  对外两个读地址：`<根>/session/<sid>/message`（工作）与 `<根>/session/<sid>/conversation`（对话）。
- **输入分离**：chat 的请求视图是**纯对话投影**——这正是用户原本要的"上下文里没有工具 / Turn"，
  它**是可达的**，且**不需要第二个存储**。
- **分界单点**：`conversation_view` 是唯一判据，三处共用（上面那张表）。

> 于是"两个不同的会话历史"这件事得到澄清：它们**不是两份历史**，而是**一份历史的两个读法**。
> 用户视角（对话线）与机器视角（工作线）本来就该不同，但不同的是**读法**，不是**记录**——
> 记录只有一份，才不会出现"两份记录互相不一致"。

#### 交叉点是固有的，拆不掉

两条线里**只有一类节点同时属于两边**：**worker 的正文**。它既是对用户的回答（对话线），
又是模型的输出（工作线）。这一类拆不开，因为它就是同一批字节。

更一般地：**一条 LLM 助手消息天然同时携带「面向用户的正文」与「面向机器的工具调用」。**
因此**靠"分两份存储"分不开两条线**——那只能把这条消息劈成两半（正文进对话流、
工具调用进工作流），再由请求视图把它**拼回一条**。混乱没有消失，只是从呈现层搬到了数据层，
并新增一个 join 的正确性负担（拼不回去 = 请求非法）。

#### 保留策略不同不需要两份存储

读法是每轮计算的（"纯机械、可重放的变换 → 视图层每轮计算，
存储 = 真实历史"，见 [session/README.md](../../symbio/src/plugins/session/README.md) §3 的决策表），
所以两条线可以有各自的窗口与过滤。既有的存储裁剪（`prune_historical_tool_calls`）
**只删工具链、保留 User / Assistant 文本骨架**——对话线因此不会被裁剪打洞。

**集合扩展点是现成的**：会话是容器，其下是若干并列集合（`message` / `inbox` / `subsession` /
`workdir` / `MEMORY.md`），"哪一类集合"由**地址段**回答，机制不认识任何一类集合
（见 [node-state-streaming.md](../../symbio/src/plugins/session/docs/node-state-streaming.md) §2.1）。
若实测证明两条线必须有不同的**写入**规则（而不只是不同的读法），再加一个集合是低成本的
——但那是**需要证据**的一步，不是起点。

#### "混乱"的来源是呈现，不是存储

今天的前端把工具卡片**插进对话列表**（[node-state-streaming.md](../../symbio/src/plugins/session/docs/node-state-streaming.md)
§5.3 的请求 / 过程 / 结果三段式、§7 的折叠行），于是"我说的话"与"机器在干什么"
混在同一列里。**这个混乱今天就存在**，与 chat 无关；加了 chat 只会让那一列更乱。

因此本方案把"两条线分开呈现"列为**必须一起做的一步**（§2.5），
而不是留给前端自由发挥。

### 2.5 呈现：两个面板，不混排

| 面板 | 内容 | 依据 |
|---|---|---|
| **对话** | user 消息 + assistant 文本（chat 的判决文本 / 进度汇报 / worker 的正文） | 对话线 |
| **工作** | Turn / 推理 / 工具调用 / 工具结果（含状态与已用时长） | 工作线 |

- **工具卡片不再插进对话流**——它进工作面板。对话面板因此是一份**可以整段回读、复制、分享**的干净记录。
- 两个面板共享**同一个顺序锚点**（`ChatMessage.seq` 是节点属性，ADR-025），
  因此"这一轮工作了 3 分钟"与"我中途插了一句话"在时间轴上**可对齐**，
  但**不在同一列里混排**。
- 实时面仍然只有一条通道（`kind = "vdfs"`），两个面板按**作用域**订阅同一批变更，
  不新增第二条通道。

---

## 3. 契约：`chat/reply`

### 3.1 类型

契约住在 `symbio_core::schemas`（跨插件：session 生产、chat 消费，满足 ADR-023 的 core 准入判据）。

```rust
/// 拟新增：symbio_core/schemas/chat/mod.rs
pub struct ReplyRequest {
    pub session_id: String,
    pub intent: ReplyIntent,              // Triage | Progress
    /// 用户输入原文（`Progress` 时为 `None`）
    pub utterance: Option<String>,
    /// 本次输入是"一条新消息"还是"抽干到的一批补充"（决定措辞，不决定判决）
    pub batch: InputBatch,
    /// **已投影**的对话上下文（文本）。session 给，chat 不自己读。
    pub context_excerpt: String,
    /// 运行现状快照：**只放事实，不放判断**
    pub snapshot: RunSnapshot,
    /// 生效模型（沿用会话选定 provider；空 = 默认）
    pub provider_id: Option<String>,
}

pub enum ReplyIntent { Triage, Progress }

pub enum InputBatch {
    /// 本轮的第一批输入（会话空闲时到达）
    Opening,
    /// 工作中抽干到的一批补充（n ≥ 1）
    Supplements { count: usize },
}

pub struct RunSnapshot {
    pub status: String,                 // working / active / failed
    pub outcome: Option<String>,        // completed / aborted / failed
    pub error: Option<String>,          // 仅 failed
    pub running: Vec<RunningItem>,      // 正在跑：name / started_at / elapsed_ms
    pub done: Vec<DoneItem>,            // 本轮已完成：name / ok
    pub plan: Option<String>,           // 会话记忆或 todo 清单里的计划（若有）
    pub last_user_facing_text: Option<String>,  // 上一次对用户说的话（避免重复）
}

pub enum ReplyOutcome {
    /// 直接答完：本轮结束，**不启动工具循环**
    Answered { text: String },
    /// 需要干活：text 作为面向用户的回应，worker 接手
    Escalate { text: String },
    /// 进度汇报（仅 `Progress`）
    Report { text: String },
}
```

### 3.2 `Answered` / `Escalate` 是**判决**，不是措辞

这是整个方案的安全阀：**chat 不做承诺，它只给一个返回值，由编排层执行。**

- 返回 `Escalate` ⇒ session **必然**启动/继续 worker。不存在"chat 说我去办但没人办"。
- 返回 `Answered` ⇒ session **必然**结束本轮，不跑工具循环。
- **任何异常都落到 `Escalate`**：超时、解析失败、空文本、模型报错、插件被停用。
  fail-safe 方向是**多干一次活**，而不是静默不干。

`Answered` 的判据是**规则，不是置信度**（置信度阈值会静默漂移，规则不会）：

> 只有当答案**完全来自已提供的上下文**（对话历史、会话记忆、工作区记忆、人格设定），
> 且**不需要去看、去做**时，才 `Answered`。
> 需要读文件 / 跑命令 / 查网页 / 访问 MCP 或技能 / 需要确认现状 / 任何有副作用的动作 → `Escalate`。

判错的代价是有界的：答得不满意，用户下一句会重新触发 `Triage`。

### 3.3 chat 的上下文：三条来源，各归其主

chat 没有工具，它要回答"进度如何""刚才那个配置改了吗"，靠的全是**上下文**。
三条来源按"变化频率 / 归属"划分，**每一类都走既有的通道**——没有新收集机制：

| 来源 | 内容 | 通道 | 为什么是这条 |
|---|---|---|---|
| **我知道什么** | 人格（setting）+ 会话记忆（session）+ 智能体记忆（memory）+ 工作区记忆（memory/workspace） | `traverse(TRAVERSE_AVAILABLE_TOOLS)` → `list_system_prompts()` | 这些是**注册型**贡献。chat 与 worker 拿到的是**同一批段、同一个通道**——chat 因此天然拥有与 worker 一致的记忆与人格，不需要任何新机制 |
| **现在在发生什么** | `RunSnapshot`：`status` / 正在跑的工具 + 已用时 / 本轮已完成 / 失败原因 / 计划 | `ctx` 载荷，由 **session 投影** | 这是**每次调用都变**的值，属 `ctx`；权威状态在 session 手里，投影归它 |
| **我们聊过什么** | 对话线（§2.4），用**它自己的窗口** | `ctx` 载荷，由 **session 投影** | 同上；且对话线必须独立于工作线的滑动窗口（否则"我前面说过"会被工具轮次挤出上下文） |

**关键点：chat 取"我知道什么"用的是与 worker 完全相同的机制。**
各插件在 `traverse` 广播里经 `CapabilityVisitor::register_system_prompt` 注册自己那一段
（`memory` 两个作用域 / `model` 人格 / `session` 会话记忆 / `setting`），chat 收集一次即得。
session 的会话记忆段有两道作用域闸门（`ctx[SESSION_ID]` 缺失即不注入），
因此 chat 的 `traverse` 上下文必须带上 `SESSION_ID` / `WORKDIR` / `AGENT_ID` / `PROVIDER_ID`
——与 session 装配 `chat_ctx` 用的是同一组键（§7.3）。

**为什么运行现状不走 `register_system_prompt`**：状态不是"注册型"贡献，它每次都在变；
且它的 owner 是**节点**（ADR-015 / ADR-025）。把它做成一个收集接口（`StatVisitor`）就是给
同一份事实造第二个来源，必然漂移——表现是"chat 说的和界面上显示的不一样"。
（两种做法的完整对照见 §2.4。）

**为什么对话线不由 chat 自己去读**：读一次 VDFS、维护一份缓存、再定一套"读到一半状态变了怎么办"
的规则——三样都是同一份事实的第二个来源。session 在调用那一刻从内存权威状态投影，零回读、零缓存。

**代价**：快照与上下文片段成为契约的一部分，且**必须只放事实**。一旦放进一个判断字段
（`needs_work: bool`…），chat 就被赋予了决策权，而决策权必须留在有工具、有上下文的编排层。

### 3.4 于是"用户问进度"不需要任何新机制

`snapshot.running`（在跑什么、跑了多久）+ `snapshot.done`（本轮已完成什么）+
`snapshot.last_user_facing_text`（上次说过什么，避免重复）+ 对话线 + 记忆，
足以让 chat 说出一句**有依据**的话：

```text
"已经查完代码（读了 3 个文件），正在跑测试，跑了 40 秒，大概还要一两分钟。"
```

这正是 `Triage` 的 `Answered` 分支——**它不需要工具，因为它要的事实已经在上下文里。**

### 3.5 产物如何落地

chat 产出的**文本**由 session 写进转写（唯一写入者不变）。三种产物两个标记：

| 产物 | 节点形态 | `meta.surface` | `meta.exclude_from_context` | 进请求包？ |
|---|---|---|---|---|
| `Answered.text` | `assistant` / `text` | `chat` | — | ✅ 它就是这一轮的答复，是对话内容 |
| `Escalate.text` | `assistant` / `text` | `chat` | `true` | ❌ 界面开场白，非模型的对话内容 |
| `Report.text` | `assistant` / `text` | `chat` | `true` | ❌ 进度汇报是给用户看的 |

两个标记回答两个不同的问题，**不要合并**：`surface` 答"谁产出的"（供前端轻量渲染），
`exclude_from_context` 答"模型该不该看到"。

**`Escalate` / `Report` 必须剔除**，硬理由有两条：① 它们是**面向用户的界面文本**，
不是"模型的对话历史"；② `Escalate` 紧跟用户消息，若进请求包，线上会出现连续两条
`assistant`——Anthropic 一类协议要求交替，会直接 400。

剔除点在 **model 插件**（[message_builder.rs](../../symbio/src/plugins/model/message_builder.rs)
的 `flatten_chat_messages`），与 `compression` 节点的处置同层——「给模型的输入按模型的视角投影」
本来就归它。

**落点**：工作中产生的节点（补充应答 / 进度汇报）挂在**在途 Turn 根之下**
（`parent_id` = Turn 根），而不是做成游离的顶层 assistant 消息——否则消息树里会出现
"不属于任何 Turn 的 assistant 节点"，而 `find_turn_user_split_idx` 与
`prune_historical_tool_calls` 都按"Turn 子树完整"来推理。无在途 Turn 时退回会话根。

---

## 4. 补充的整合（目标 2）

### 4.1 收件箱 = 输入缓冲，不是"一条消息 = 一轮"

地址与条目形状**都不变**（`<sid>/inbox/<iid>`、`InboxItem`）。变的是**消费语义**：

```text
今天：消费者 pop_front 一条 → 起一轮
本方案：抽干整队 → 合并成一条用户输入 → 交给 triage
```

于是"用户中途补充"不再各占一轮，而是**在下一轮边界被整体折进当前工作**。

### 4.2 抽干的两个时机

| 时机 | 位置 | 行为 |
|---|---|---|
| **空闲起轮** | 收件箱消费者（`drain_inbox_once`） | 抽干整队 → 合并 → 作为本轮第一条用户输入 → triage |
| **轮边界** | `run_chat_loop` 主循环**步骤 1 之后、`gate_turn` 之前** | 抽干整队 → 合并 → 追加进当前轮上下文 → triage |

**为什么轮边界只能是"工具批次收齐之后"**：`process_tool_calls_async` 是串行阻塞的，
回到循环顶部时本批工具结果**必然已齐**。若在别处插入用户消息，就会插在
`assistant(tool_calls)` 与其 `tool` 结果之间——OpenAI / Anthropic 都会 400。

**轮结束仍有条目** → 开新轮（今天的 FIFO 兜底，不变）。

### 4.3 合并成**一条**用户消息

抽干到的 n 条按原顺序拼成**一条** `role = user` 的消息：

```text
msg_type = text
meta.supplement = true
meta.supplement_ids = [<iid>, …]     // 可追溯回原始条目
meta.supplement_count = n
```

**为什么必须合并而不是逐条追加**：**存储裁剪**（`prune_historical_tool_calls` 的 `keep_turns`
分水岭，见 `symbio/src/plugins/session/chat_session/write.rs`）按 **User 消息计数**。
逐条追加会让 n 条补充吃掉 n 份保留预算，把更早的轮次提前挤出窗口。
合并成一条，`n` 条补充在计数上**仍是一轮**，与用户心智一致。

> ⚠️ **两处计数口径不同**：存储裁剪按 **User 消息**计数；请求视图窗口
> （`apply_layered_sliding_window`，`symbio/src/plugins/session/context/window.rs`）
> 按 **ToolCall** 计数，不受补充条数影响。只有前者需要靠"合并成一条"来守。

补充消息的落点：**顶层**（`parent_id = None`），与普通用户消息同形。已核实
`find_turn_user_split_idx` 只回看 `messages[..turn_idx]` 且要求 `parent_id.is_none()`，
补充写在 Turn 根**之后**，因此不会影响本轮起点判定。

---

## 5. 中途汇报（目标 3）

### 5.1 触发是状态派生的，不是模型自发的

```text
is_working
  && now - last_user_facing_at >= progress_interval_ms
  && progress_reports_this_turn < max_progress_per_turn
  && tool_rounds >= progress_min_rounds
        ⇒ route(chat/reply, Progress) → 写 Report 节点
```

新增两项会话内状态（`ActiveSessionStateInner`）：`last_user_facing_at`（最后一次对用户
说话的毫秒时间戳）、`progress_reports_this_turn`（本轮已汇报次数，随 Turn 复位）。

### 5.2 复用既有的 15s 调度 tick

不新建定时器：`session/heartbeat` 的调度循环本来就在扫"本 store 全部会话 + 判 `is_working`"
（见 [heartbeat-mechanism.md](../../symbio/src/plugins/session/docs/heartbeat-mechanism.md)）。
在同一个 tick 上增加一条"该不该汇报"的判定即可。心跳触发的是**一轮**，
进度汇报只**写一条节点**——两者共用扫描，不共用动作。

`is_working` 是进程内状态，因此只有拥有该会话的进程会汇报，天然不重复。

### 5.3 频率护栏

`progress_interval_ms`（默认 45s）、`max_progress_per_turn`（默认 5）、
`progress_min_rounds`（默认 1，避免刚开工就汇报）。汇报不重试、失败即跳过。

> 汇报与前端已有的派生标签**不重复**：标签是机械的、逐工具的（"正在调用 X · 运行中 47s"），
> 汇报是一句综合的话（"代码查完了，正在跑测试，大概还要一分钟"）。

---

## 6. 直接回答（目标 1）

### 6.1 落点是主循环内的两个调用点

不新增旁路——triage 是**主循环前步骤**与**轮边界**上的一步，因此完整复用
`WorkingGuard` / `AbortGuard` / `StopSignal` / 增量落库锚点：

```text
run_chat_loop
  ├─ 前步骤 P0：本轮用户消息 → triage
  │     ├─ Answered → 写答话节点 → finish_turn(TurnExit::Completed)   ← 干净收尾，不开循环
  │     └─ Escalate → 写首响节点 → 继续
  ├─ loop 步骤 1 … 步骤 7
  │     └─ 步骤 1 之后：抽干补充 → 合并 → 追加进上下文 → triage
  │           ├─ Answered → 写答话节点 → 继续（worker 继续自己的活）
  │           └─ Escalate → 写"收到补充"节点 → 继续
  └─ finish_turn（唯一收尾点，不变）
```

### 6.2 会话状态与前端行为

`is_working` 仍在 triage 之前置位（前端照常显示"处理中"）；`Answered` 路径经
`finish_turn` 正常回落 `active`。前端**零改动**即可正确工作——`Answered` 就是一次
"很短的一轮"，节点状态自洽。

---

## 7. 避免插件之间直接依赖

### 7.1 四条既有通道，本设计用哪条

| 通道 | 形状 | 本设计 |
|---|---|---|
| `route(path)` | 请求-响应（可流式） | ✅ session → chat |
| VDFS 地址写入 | `write` / `action`，无调用方 | ✅ 用户 → inbox（现状） |
| `event_bus` + `vdfs/watch` | 进程内广播 | ❌ 不用（chat 无副作用，不观察） |
| `traverse` + visitor | 广播收集 | ✅ chat 用它取 `ModelProvider` |

### 7.2 调用形状

```rust
// session 侧（拟新增，形制照抄 agent_run → session/chat/send）
let mut c = ctx.fork();
c.set(PATH, ROUTE_CHAT_REPLY.to_string());        // "chat/reply"，常量进 symbio_core::plugin::route
c.set(SESSION_ID, session_id.clone());
c.set_payload(ReplyRequest { … })?;
let out: ReplyOutcome = parent.clone().route(c).await?.payload()?;   // parent = worker 容器（Weak<dyn Plugin>）
```

调用方**不 `use` 被调方任何符号**：session 只写路径常量，chat 只实现 `Plugin`、
不知道谁在调它。`agent_run` 调 `session/chat/send` 已经是这个形态——本方案把它写成明文规则（[ADR-042](../decisions/core.md#adr-042-跨插件调用一律经容器-route--路径常量不持有对方类型)）。

### 7.3 chat 怎么拿到 `ModelProvider` 与记忆

chat **不**引用 model / memory / setting 任何一个插件（model 无自有路由）。
它在 `ctx` 里带上 `PROVIDER_ID` / `SESSION_ID` / `WORKDIR` / `AGENT_ID`，
对自己调**一次** `traverse(TRAVERSE_AVAILABLE_TOOLS)`，这一次广播同时收到两样东西：

- `get_model_provider()` → 当次生效的 `Arc<dyn ModelProvider>`（model 插件注册，单槽覆盖）；
- `list_system_prompts()` → 人格 / 会话记忆 / 智能体记忆 / 工作区记忆的全部段。

与 session 的取法是**同一条路、同一组键**（`session/capabilities.rs` 的 `collect_capabilities`），
只是各取各的、互不共享状态——**重复的是机制调用，不是数据**。

---

## 8. 不变量与失败处置

### 8.1 新增不变量

1. **转写的写入者仍然只有一个**（session）。chat 产出的是**文本**，不是节点。
2. **`chat` 不注册任何 `Capability`**：模型的工具集里不可能出现 chat。
3. **`chat` 无副作用**：不写任何地址、不调任何 `route`、不起后台任务。
4. **`RunSnapshot` 只放事实**：任何判断字段都不得进入。
5. **异常一律 `Escalate`**：fail-safe 方向是多干一次活，不是静默不干。
6. **抽干只能发生在工具批次收齐之后**（轮边界），否则会插在 `tool_calls` 与其结果之间。
7. **抽干到的 n 条补充合并成一条用户消息**：轮次计数（窗口 / 裁剪分水岭）保持"一轮"语义。
8. **`meta.exclude_from_context` 的节点必须落库**：用户看到过的话都算数，重开会话不能消失。
9. **`Answered` 的答话按普通消息进上下文**（它是这一轮的答复）；只有 `Escalate` / `Report` 剔除。
10. **话语权限**：`Escalate` / `Report` 可以承诺"我接下来会做 / 正在做"（编排层保证），
    **不得承诺具体结果**；`Answered` 就是答案本身，不含承诺。
11. **会话只有一份存储**：对话线与工作线是它的**两个读法**，不是两份数据。
    一条助手消息横跨两条线（正文 vs 工具调用）是**固有属性**，不靠分存储消除。
12. **两条线在呈现上不得混排**：工具 / 推理节点不进对话面板。这是本次改造**必须一起做**的一步
    ——否则 chat 的加入只会让现有那一列更乱。
13. **chat 的"我知道什么"只走 `register_system_prompt`**（与 worker 同一通道），
    不得为记忆 / 人格另建收集接口。
14. **对话线的窗口独立于工作线**：工作线的滑动窗口 / 骨架化 / 淡化不得影响对话线——
    否则"我前面说过"会被工具轮次挤出上下文。
15. **分界规则是单点纯函数**：`conversation_view`（§2.4）是"某节点属于哪条线"的唯一判据，
    三处共用（chat 上下文 / 前端对话面板 / 工作线请求视图）。
    **chat 的请求视图里不得出现 `tool_call` / `role = tool` / Turn 容器 / 推理节点**——
    这条由投影函数保证，**不得**依赖 chat 的提示词（提示词约束不了一个已经进包的东西）。

### 8.2 失败处置

| 失败 | 处置 |
|---|---|
| chat 超时 / 报错 / 返回空 | 落到 `Escalate`（不阻断、不重试） |
| chat 插件被停用 / `enabled: false` | 全部输入直接 `Escalate`，行为与改造前**逐字一致** |
| 写节点失败 | 记 warn，不影响收件箱条目（条目才是交付物） |
| 同一会话并发多次 chat 调用 | 按会话去抖（`max_inflight = 1`），后到者丢弃 |
| 进度汇报失败 | 跳过，下个 tick 再看 |

---

## 9. 配置面

```yaml
# <根>/chat/PLUGIN.yml（配置就是插件目录里的 PLUGIN.yml，无第二条配置协议）
chat:
  enabled: true              # 总开关；false ⇒ 全部 Escalate，行为与改造前一致
  triage: true               # 目标 1：直接回答 / 升级的判决
  provider_id: null          # 空 = 沿用会话选定模型；可指定更快更便宜的
  timeout_ms: 8000           # 超时即 Escalate
  max_chars: 200             # Escalate 文本要短
  max_inflight: 1
  # 对话线的窗口（§2.4）——独立于工作线的 context_messages，
  # 否则"我前面说过"会被工具轮次挤出上下文
  conversation_messages: 20

progress:                    # 目标 3
  enabled: true
  interval_ms: 45000
  max_per_turn: 5
  min_rounds: 1

supplements:                 # 目标 2
  enabled: true              # false ⇒ 退回"一条消息 = 一轮"（今天的行为）
  max_per_drain: 20          # 单次抽干上限（超出者留队，下一轮边界再抽）
```

配置走既有的 `capability_announce_configurable`，设置页自动出现。

---

## 10. 实施批次

| 批次 | 内容 | 验收 |
|---|---|---|
| **P0 · 先量** | 埋点：`用户消息入队 → 第一条面向用户文本落库` 的时延分布（p50/p95）；`简单问题` 与 `有工具调用` 分档 | 有一张数。没有它无法证明收益 |
| **P1 · triage** | `chat` 插件骨架 + `chat/reply` 契约 + `ReplyOutcome` + session 前步骤 triage + `meta.exclude_from_context` 剔除（model 插件） | 关闭开关行为与今天逐字一致；开启后简单问题不再进工具循环 |
| **P2 · 补充整合** | `drain_inbox` 抽干整队 + 轮边界抽干点 + 合并成一条 + `meta.supplement` | 连续三条补充只产生一轮整合处理；工具批次完整性用例（不插在 tool_calls 与结果之间）绿 |
| **P3 · 两条线分开呈现** | 前端两个面板（对话 / 工作）+ 对话线读地址 `<sid>/conversation`；工具卡片移出对话列表 | 对话面板可整段复制且不含工具卡片；两个面板的时间轴可对齐 |
| **P4 · 中途汇报** | 复用 15s tick 的进度判定 + `last_user_facing_at` / `progress_reports_this_turn` | 长任务中途出现汇报节点；护栏生效（频率、上限） |
| **P5 · 调优** | 独立模型 / 温度 / 提示词模板 | 可 A/B |

P1 与 P2 是**必须做对**的两批（对应目标 1 与目标 2）；P3 是**呈现上的必要条件**
（§2.5：不分开呈现，chat 只会让现有那一列更乱）。

---

## 11. 待定项

1. **插件名**：`chat` 在本仓已有"会话轮次"的含义（`session/chat/send`、`chat_loop`、
   `chat_message`），新插件用同名会让它承担第二个意思。建议 `dialog`；路由上不冲突，
   故可议。**地址一旦定下就是身份**（ADR-032），须在写第一行代码前定。
2. **`Answered` 是否也写用户消息**：本方案为"写"（用户说过的话必须落库）。
   若实测发现模型频繁与既有历史重复，再考虑只落答话。
3. **抽干是否需要"静默窗口"**：用户连续敲三条补充，若第一条到达时恰好是轮边界，
   会先整合一条、余下两条下一轮边界再整合。是否值得加一个极短静默窗口（如 300ms）
   等齐再抽，由 P2 实测决定。
4. **`Progress` 与 `Escalate` 是否合并成一次调用**：两者都在轮边界附近，理论上可合并
   为一次 `chat/reply`。分开是因为触发条件不同（汇报由定时器触发，triage 由输入触发）。
5. **对话线是否只收顶层消息**：本方案取"user 消息 + assistant 文本节点（含 Turn 内的）"，
   判据是**用户视角**（用户看得到的就是对话）。若实测发现 Turn 内的规划性文本污染对话线，
   再收紧为"只收顶层"——那是投影的一个参数，不需要改存储。
6. **对话线的窗口大小**：`conversation_messages` 取多少轮（工作线默认 `context_messages = 6`），
   由 P1 实测决定；对话线的记忆由会话记忆（`MEMORY.md`）兜底。

---

## 12. 边界（不做什么）

| 不做 | 一行理由 |
|---|---|
| chat 直接写转写 | 转写只有一个写入者（ADR-020），两个写入者会让 `seq` / 合帧 / 终态收敛同时失去单点保证 |
| chat 自己订阅状态、自行决定何时播报 | 触发条件归编排层（状态派生），chat 只负责措辞 |
| 新建 `StatVisitor` | 运行现状的 owner 是节点（ADR-015 / ADR-025），第二个来源必然漂移 |
| 为 chat 另建记忆 / 人格收集通道 | `register_system_prompt` 已是唯一注册入口，与 worker 共用同一批段 |
| 对话线与工作线各存一份 | 一条助手消息会横跨两流（正文 vs 工具调用），请求视图必须再拼回来——混乱从呈现层搬到数据层 |
| 把"两条线"实现成两处各自的过滤条件 | 分界规则必须是单点纯函数（§2.4），散落三处必然错位：界面看得到、chat 看不到 |
| 靠提示词让 chat"别看工具结果" | 约束必须落在投影函数上——进了包的东西，提示词管不住 |
| 两条线在同一个列表里混排 | 工具卡片插进对话是**今天就有**的混乱来源，与 chat 无关；必须一起改 |
| 把 `chat` 暴露成 LLM 工具 | 模型本来就会产出面向用户的正文且那条路是流式的，做成工具等于同一件事两条路 |
| 把 `session` 改名 `worker` | 插件名 = 地址（挂载点 / 路由前缀 / 配置目录）；且 `worker` 已被 composite 容器占用。角色名写进 `PLUGIN.yml` 的 `plugin_title` |
| 让 chat 决定"要不要干活" | 它无工具、无执行上下文；决策权归有工具、有历史的编排层 |
