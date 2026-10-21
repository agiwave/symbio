//! turn 运行器 —— v2 深度档的**执行引擎**（原住 `symbio_core::actors`，2026-10-09 下沉至此）。
//!
//! ## 为什么它不在 core
//!
//! `core-export-audit` 的 C-003 问的是「**谁依赖它**」，判据是**生产模块数**（测试不算，
//! 口径见 `scripts/core-surface.mjs`）。本运行器的生产消费方**只有本插件**：core 内除
//! `actors` 自身外**零使用**——`adapters/mod.rs` / `adapters/provider_adapter.rs` /
//! `event/mod.rs` 里出现的名字全在**文档注释**里，不是依赖。
//! 按 `symbio_core/README.md` §4 四问第 1 问「只有一个依赖方 ⇒ 下沉回那个模块」，
//! 它住 core 是**放错了层**：那是**实现**（流循环 + 重试 + 落格顺序），不是契约。
//!
//! ## 它不是冻结契约（README §1.2 曾把它登记错）
//!
//! README §1.2 把它登记为 `plan/01 §4` 的「冻结契约名」，但**那个出处是假的**——
//! `grep -c TurnRunner docs/004-plan/001-核心架构.md` = 0。真正的冻结锚点
//! （[plan/03 §1](../../../../docs/004-plan/003-演进与验证.md "PL-003-演进与验证")）是：F1 `Store` 签名 /
//! F2 `Projection` 构造形状 / **F3 `ActorSpec` 字段集 + 档位→令牌映射** /
//! F4 三条不变量 / F5 三个封闭集合 / F6 参数表键集。**`TurnRunner` 不在其中**，
//! 所以这次下沉**不改 F1–F6**，按 [plan/03 §2](../../../../docs/004-plan/003-演进与验证.md "PL-003-演进与验证")
//! 的判层不是架构变更。
//!
//! ## core 侧留下了什么
//!
//! `ActorSpec`（F3）/ `Pattern` / `Scope`（F5）留在 `symbio_core::actors`——它们是锚点
//! 与封闭集合，本文件按名引用（`crate::symbio_core::…`，与其它插件同一条路径）。
//!
//! ## 落格纪律（随代码搬来，未改一字）
//!
//! 「一个 turn = 用户格 opened + 至多一条收束 closed」这条不变量由本运行器守着；
//! 插件侧**不得**自己补写那两格（否则同轮两份记账的假象会回来）。

use crate::symbio_core::{
    visible_to, ActorSpec, AdapterError, Entity, Event, FullModel, LatencyTier, LlmAdapter,
    LlmTurn, PromptToolCall, Seq, Store, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_ASSISTANT_OPENING, EVENT_ASSISTANT_PROGRESS, EVENT_AUTHZ_DENIED,
    EVENT_SESSION_CONSOLIDATED, EVENT_USER_MESSAGE,
};
use std::borrow::Cow;

// ── 阶段拆分层（T-179 · B3，PL-015 §5 动作 B）──────────────────────────
// 逐步过程按阶段切：准备 → 请求 / 流式 → 工具批 → 轮边界 → 收束。
// 阶段之间只传 [`prepare::Prepared`]，不传第二份事实。
mod batch;
mod boundary;
mod facts;
mod prepare;
mod reflex;
mod settle;
mod stream;

use self::facts::*;

pub(crate) use self::boundary::{classify_round, final_answer, RoundDecision};
pub(crate) use self::prepare::Prepared;

// ── v2 会话运行时：turn 运行器（chat_loop 切换的第一块可复用件）────────
//
// 职责把 [plan/04 §2](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 的单轮流程落成
// 一个函数调用：用户消息入格 → [`Reasoner`] 生成（实测耗时）→ final / fallback
// 落格（I3：失败也必须有输出）。不变量靠构造成立：每轮独立 turn 号（N3）、
// final 溯源指向本轮用户消息（N5）、实测 cost_ms 随事件入账（ADR-044）。
// 会话轮与彩排共用本运行器：会话轮由 chat_loop 经 [`super::turn_exec`] 驱动，
// 彩排由真实端点校准使用。

/// 一轮的**调度输入**（会话轮与彩排共用的入参包）。
///
/// `Debug` **手写**：`inject` 是闭包（`dyn Fn` 不实现 `Debug`）。那一行打印成
/// `inject: <闭包>` 而不是把闭包捕获的东西摊开——闭包捕获什么与这个入参包的
/// 可读性无关，而把它摊开会**打印用户对话正文**。
#[derive(Clone)]
pub struct TurnInput {
    /// 本轮 turn 号（调用方保证单调递增——N3 的轮次锚）。
    ///
    /// 续写轮（[`Self::resume`] 为 `Some`）时，调用方填的是**被续写轮**的号
    /// （已开未收束的那一轮），不是新号。
    pub turn: u64,
    /// 用户发言。续写轮不新开用户格，此字段不参与入格（用户没再说话）。
    pub text: String,
    /// **本轮的中止信号**（ADR-052：中止是事实的**状态**，不是「什么都没发生」）。
    ///
    /// ## 为什么运行器要拿它
    ///
    /// 中止可以在**用户格落完之后、第一次 LLM 请求之前**到达（用户按下停止的
    /// 那一刻恰好落在闸门与请求之间）。若拦在进运行器**之前**（`TurnExit::Aborted`），
    /// 运行器就从不落 `user.message` 格：WAL 全空，中止这一轮**一条事实都没有**
    /// （e2e t7 钉的是这件事：`v2-events.wal` 不存在，恢复后重建的历史里看不到
    /// 这轮说过什么）。而 ADR-052 决策 1 要求「一轮的产出无论完成、失败、被中断
    /// 还是悬在等待用户，都入格」。
    ///
    /// 形状 = 检查点在运行器内：落完用户格后、发第一个请求前检查一次，
    /// 已中止则直接走**中止收束格**（`chat.assistant.final` + `status: "aborted"` +
    /// `terminated`）——与 adapter 中途返回 `AdapterError::Aborted` 落的是同一格。
    ///
    /// `None` = 调用方不提供信号（单测 / 无中止语义的路径），行为与接线前逐字一致。
    pub abort: Option<crate::symbio_core::ExecAbortSignal>,
    /// **本轮已执行的一次性阶段**（`turn.stage` 草稿，ADR-107 判据 2）。
    ///
    /// 由编排层在阶段执行点产出（判决 / 召回 / 压缩），运行器在**用户格之后**、
    /// 开场白与收束之前落格——断言类事件必须带溯源锚点（I2），而那只有落完用户格
    /// 才有。续写幕重跑同一轮时撞幂等键，`stage_events::land` 按「已做过」吸收。
    ///
    /// 空表是常态（没有一次性阶段执行过）；它不是 `Option`——「没有阶段」与
    /// 「有阶段但清单为空」在这里是同一件事。
    pub stages: Vec<super::stage_events::StageFact>,
    /// 本轮装配进哪一时延档（调度决定是数据，ADR-044）。
    pub tier: LatencyTier,
    /// 对话窗口（最近的 turn 数，**含当前轮**）；`None` = 全量。
    ///
    /// 会话轮必填——WAL 里的事实只增不减，不带窗口的 prompt 会随会话
    /// 无界增长；窗口值由调用方的窗口配置给出（如 `context_messages`）。
    pub window_turns: Option<u64>,
    /// 续写锚点：`Some` ⇒ 本轮**续写**一个已开未收束的轮次（审批 / 问答恢复），
    /// `None` ⇒ 新开一轮。见 [`TurnResume`]。
    pub resume: Option<TurnResume>,
    /// **本轮以哪个主体的身份记账**——收束 / 产物事件的 `actor`、窗口可见域的
    /// `viewer` 都取它（[plan/11 批 1](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
    /// ①：`"session/main"` 从字面量变成**入参**，`ActorSpec` 首次在生产构造）。
    ///
    /// 身份是数据不是常量：同一个会话引擎跑在哪个 agent 上，由**调用方**说了算
    /// （`plugins/session/turn_exec` 从会话元数据派生）。S08 §4 的平凡值是
    /// `agent:main`——所有主体同一身份时退化成单主体，与接线前逐字一致。
    pub actor: ActorSpec,
    /// **请求级前缀**：排在基线 prompt **之前**的一段文本（通常是多行），
    /// `None` / 空串 = 不加（生产里的绝大多数轮次）。
    ///
    /// ## 它解决的是什么
    ///
    /// v2 运行器的 prompt 是**一条** user 消息（`ProviderLlmAdapter::generate_turn`
    /// 把渲染结果整段发出），而请求视图层（`build_request_view`）产出的长期记忆
    /// 召回段 / 就绪任务集段 / 委派者真源段是**独立的消息**。这三段若不显式带进来，
    /// 就**静默**从模型眼前消失——事实照样入格、`session/stats` 照样有数，
    /// 只有模型看不见（t29 / t35 钉住这类静默消失）。
    ///
    /// ## 为什么进 `TurnInput` 而不是让调用方拼进 `text`
    ///
    /// `text` 是**用户发言**，它要落进 `user.message` 的载荷、并参与记忆编码与
    /// 技能命中判定。把前缀拼进去会让事实格里存进一段不是用户说的话——记忆编码
    /// 照着它固化、技能命中拿它比对，两处一起记错同一件事。前缀是**请求级的**，
    /// 不进事实源，所以它自己占一个字段。
    ///
    /// ## 落位：基线**之前**，不进「就地累积」
    ///
    /// 三段都是 `build_request_view` 的**置顶**段，语义上在对话之前；追加到基线
    /// 末尾（`exchange` 那条路）会把它们排到本轮发言之后，模型先答后看指令。
    /// 跨轮累积也一并排除：它们每轮重算，累积会让上一轮的前缀留在下一轮的上下文里。
    pub prefix: Option<String>,
    /// **轮内折进的补充**：工具循环每跑完一轮就问一次「用户有没有补充」，
    /// 有就折进本轮（成为一格事实 + 进下一次请求的消息数组）。
    ///
    /// ## 它解决的是什么（缺口 3）
    ///
    /// 用户在助手干活途中补了一句话。运行器的工具循环里**没有**抽干补充的代码
    /// （抽干在插件侧，见 `supplements` / `inject` 的分工），补充只经本回调进来。
    /// 不接这个回调，于是：
    ///
    /// - 用户的话被收件箱队列**吸走**了（队列已消费、没人用它），
    /// - 模型**看不见**（prompt 从事实网格投影，而它从未入格），
    /// - 而这一切**完全静默**：不报错、不告警、既有断言全绿。
    ///
    /// 「会话接受了一句用户话，但它从未成为事实」。
    ///
    /// ## core 只知道「加消息」，不知道「加的是什么」
    ///
    /// **为什么是回调而不是「调用方开跑前把补充全取出来传进来」**：补充是**轮内**到达的
    /// ——工具跑到一半用户才说话。调用方若只在开跑前取一次，之后到达的那些仍然丢，
    /// 而那恰恰是补充最常见的形态（用户在等结果时插话）。回调把「什么时候问」放在
    /// **唯一知道循环边界的地方**（`run_with_tools` 的每次迭代末尾），插件侧不必把
    /// 循环搬到外面。
    ///
    /// 但 core **不该**知道那是「收件箱补充」——那是插件侧的概念（收件箱、
    /// `merge_supplements`、`ChatNode` 合并都在 `plugins/session`）。于是这个口
    /// 收的是**已经折好的消息**（`PromptMessage` 本来就存在），落格也在插件侧做
    /// （`turn_exec` 自己开着 `EventWalStore`）。
    ///
    /// 回调返回**要加进下一次请求的消息**；返回空 vec 是绝大多数轮次的情形。
    ///
    /// ## 为什么是闭包而不是 trait
    ///
    /// core 里为一个「取几个消息」的动作造 trait + 默认实现 + 关联类型，是**用抽象
    /// 换不来任何东西**：接口面变大、可读性变差，收益是零。闭包就够。
    ///
    /// ⚠️ 调用方负责**取走语义**（同一条不会被注入两次）——core 不做去重。
    pub inject: Option<RoundInjector>,
    /// **本轮已有定稿答话**：`Some(答话)` ⇒ 不调模型，这段文字直接成为本轮输出
    /// （`chat.assistant.final` 那格照常落）。
    ///
    /// ## 它解决的是什么（缺口 5）
    ///
    /// `classify` 判 `Answered` ⇒ `compose` 出好答话 ⇒ 本轮**一个模型请求都不该
    /// 发**。而 prompt 从事实网格投影 ⇒ 答话若只 push 进 `context.messages` 后
    /// 收尾，那次问答在网格里**完全不存在**（用户格与收束格都由本运行器落，
    /// 该轮不进运行器就一格都没有）⇒ 下一轮模型不知道自己刚回答过。
    ///
    /// ## 为什么由 core 承接而不是插件自己落那两格
    ///
    /// 「一个 turn = 用户格 opened + 至多一条收束 closed」这条不变量由运行器守着。
    /// 插件自己补写就成了**第二个轮次事实写方**（同轮两份记账的假象）。所以走这里：
    /// 答话给运行器，格仍由它落。
    ///
    /// ## `model` 载荷字段留空
    ///
    /// 这一轮没有模型参与，`chat.assistant.final` 的 `model` 载荷记**空串**而不是
    /// 任何模型的 id——记一个会变成「这次请求用的是那个模型」，而事实相反。
    pub final_reply: Option<String>,
    /// 定稿答话**对话面节点**的身份与标记（仅 `final_reply` 有值时消费）。
    ///
    /// ## 为什么答话要带这三样，而不是只有正文
    ///
    /// `Answered` 的答话是一条**根级**对话面节点（`parent_id = None`，带
    /// `meta.surface = "reply"` / `meta.reason` / 可选的 `meta.exclude_from_context`）。
    /// 收束格若只记正文（`final_reply`），重建便**折不出这条节点**——实测
    /// t20 / t21 / t22 / t25 全部「0 条对话面节点」。节点形状是事实的一部分
    /// （T-052 原则），故随收束格一并入格（WAL 为唯一事实源，ADR-049）。
    pub dialog: Option<DialogFacts>,
    /// **对话面开场白**（T-056）：`Escalate` 首响正文（`Some` = 本轮有一句开场白）。
    ///
    /// 与 [`Self::final_reply`] 的区别只在**时机**：答话是干活干完之后的收束，
    /// 开场白是干活**开始前**说的一句（本轮随后照进工具循环）。两者都产出一条
    /// **根级**对话面节点（同 `dialog_node` 形状），但收束格那格（`chat.assistant.final`）
    /// 是 N3「每轮至多一条」，开场白不能挤进去——故它单立一格
    /// （`chat.assistant.opening`，`turn × asserted`，见 core 常量文档）。
    ///
    /// 承载事实由运行器落（与收束格同一条纪律：节点诞生归运行器，`apply` 不另落）。
    /// 载荷带 `opening_dialog` 的 `id` / `reason` / `exclude_from_context`。
    pub opening_text: Option<String>,
    /// 开场白节点身份与标记（仅 [`Self::opening_text`] 有值时消费）。
    pub opening_dialog: Option<DialogFacts>,
    /// **本轮 Turn 根节点 id**（落库时 `into_messages(root_id)` 的入参）。
    ///
    /// 由**调用方分配**（`chat_loop/inputs.rs` 的 `llm_short_id()`，那里是唯一的
    /// 分配点），运行器只把它记进收束事件的载荷——「谁分配谁记录」要求事件载荷与
    /// 落库回包用的是**同一个 id**，两边各自现造就必然对不上（重建出的 Turn 根与
    /// 落库的 Turn 根是两个节点）。
    pub turn_root_id: String,
    /// 本轮**用户消息节点 id**（`user.message` 载荷记它）。`None` = 本轮没有用户
    /// 节点（续写轮不新开用户格，不消费）。
    ///
    /// 与 [`Self::turn_root_id`] 同一条纪律：id 由调用方生成（用户消息在进运行器
    /// 之前就已存在），这里只负责**记**。
    pub user_node_id: Option<String>,
    /// 本轮**用户消息节点的 `meta`**（`user.message` 载荷记它；`None` = 无 meta）。
    ///
    /// ## 为什么 `meta` 也是事实的一部分
    ///
    /// 用户消息不只有正文：折进的那一条（合并补充）带 `meta.supplement*` 三键——
    /// 前端据它认出「这是一次折进」，重建据它复刻节点形状。这些键若不随事实
    /// 入格，重建出的用户节点就**丢了这些键**，
    /// 于是同一条发言在实时面与重建面是两个身份（t19 的空闲抽干线照出来的正是这一处）。
    /// 与 `id` 同属「节点身份/形状是事实的一部分」（T-052 原则）。
    pub user_meta: Option<serde_json::Value>,
    /// **流式期已分配的节点身份**（正文 / 推理子节点的 id 与推理正文）——收束事件
    /// 的载荷要记它们（WAL 载荷补全：消息节点 id 入格，重建才无损）。
    ///
    /// 为什么是闭包而不是直接给值：节点在**流式期间**由 UI 桥的首片分配，而收束
    /// 事件写在流结束之后——运行器与桥之间隔着 `dyn DeltaSink`，拿不到桥的内部
    /// 状态。闭包把「取此刻的节点事实」留给唯一知道它的那一侧（与
    /// [`Self::inject`] 同形态的理由：core 不认识桥）。
    pub nodes: Option<NodeFactsFn>,
}

/// 定稿答话**对话面节点**的身份与标记（见 [`TurnInput::dialog`]）。
///
/// 三样缺一不可：`id` 是节点身份（前端按 id 合并、`message.edited` 按 id 定位）、
/// `reason` 是上游判决原样透传（`meta.reason`）、`exclude_from_context` 决定这句
/// 话进不进模型请求包。重建按这三样经**同一个构造器**（`compose::dialog_node`）
/// 复刻，形状漂移因此不可能。
#[derive(Debug, Clone, Default)]
pub struct DialogFacts {
    /// 对话面节点 id（写侧 `dialog_node` 分配的 `id`）。
    pub id: String,
    /// 理由码（`meta.reason`；上游判决原样透传）。
    pub reason: String,
    /// 是否不进模型请求包（`Escalate` 首响为真，`Answered` 答话为假）。
    pub exclude_from_context: bool,
}

/// 流式期已分配的节点事实（见 [`TurnInput::nodes`]）。
///
/// 四个字段一起取：收束 / 进展载荷要的是「这一轮的正文 / 推理节点是哪两个、两段正文
/// 是什么」——拆成四个回调只会让取值点变四处。
#[derive(Clone, Default)]
pub struct StreamNodeFacts {
    /// 正文子节点 id（`TurnOutput::response_text_child_id` 同源）。
    pub text_id: Option<String>,
    /// 正文累积（**半截也是事实**：中止时它就是要入格的那半句话）。
    pub text: Option<String>,
    /// 推理子节点 id（`TurnOutput::reasoning_child_id` 同源）。
    pub reasoning_id: Option<String>,
    /// 推理正文（reasoning-only 判据在重建侧要复算，故正文也要记）。
    pub reasoning: Option<String>,
}

/// 「取此刻的流式节点事实」的取数口（见 [`TurnInput::nodes`]）。
/// 与 [`RoundInjector`] 同一条「闭包而不是 trait」的判据：一个取值动作不立 trait。
pub type NodeFactsFn = std::sync::Arc<dyn Fn() -> StreamNodeFacts + Send + Sync>;

/// 一次工具调用的事实形状（收束 / 进展载荷的 `tool_calls` 条目）。
///
/// `result_id` 是**引用**而不是结果正文：工具结果正文归 `artifact.added`（一个事实
/// 一个 owner），这里只记「结果落在哪个节点」。配不上结果的调用记 `None`——不静默
/// 补空串（空串读出来是「结果节点 id 是空的」，而事实是「还没有结果」）。
#[derive(Debug, Clone)]
pub struct ToolCallFact {
    /// ToolCall 节点 id（`llm_build_tool_call_nodes` 的同源 id）。
    pub id: String,
    /// 工具名（LLM 可见名）。
    pub name: String,
    /// 定稿参数文本（JSON 文本 / 解析失败的残破原文，与
    /// [`crate::symbio_core::node_build::tool_call_args_text`] 同一取法）。
    pub args: String,
    /// provider wire id（`tool_call_id` 字段——节点身份的一部分）。
    pub wire_id: String,
    /// 结果子节点 id（`None` = 尚无结果：无分发通道 / 等待用户 / 恢复路）。
    pub result_id: Option<String>,
}

/// 「下一轮要加哪些消息」的取数口（见 [`TurnInput::inject`]）。
///
/// 每次工具循环迭代末尾调一次。
/// 名字叫 `Round` 而不是 `Turn`：`Turn` 在本仓已被 `llm` 域占用（`TurnInput` /
/// `TurnOutput` / `TurnResume`），而 core-naming-audit 的 N-003 判「撞别的域前缀」
/// 即红。顺带语义更准：注入发生在**工具轮的边界**，不是轮的内部。
///
/// 入参 = 本轮用户格的 seq（`None` = 本轮没有用户格，续写轮的极端情形）。
///
/// **为什么是 `Future` 而不是裸 `Vec`**（缺口 4 的实测）：同一个挂点上还挂着
/// 「轮边界汇报」——它要 `await` compose 的路由（插件间调用）。同步闭包只能在
/// `block_in_place` 里 `block_on`，而**单线程 runtime 上那样会 panic**（tokio 明令
/// 禁止），于是汇报会变成「多线程能用、单线程静默失效」——那正是本仓最恨的那种
/// 形态：行为随部署方式变，且没有任何信号。异步闭包没有这个问题：core 在自己的
/// async上下文里 `.await`，调用方的实现随便异步还是同步。
///
/// **为什么把锚点传进来而不是让调用方自己找**：用户格是**运行器**落的，调用方
/// 那边的快照取在它**之前**——于是「自己去网格里找本轮用户格」在开跑那一刻
/// 必然找不到，于是补充格永远没有溯源（而 I2 要求断言类事件必须带溯源）。
/// core 手里有那个 seq，给出去比让调用方猜更省事也更可靠。
pub type RoundInjector = std::sync::Arc<
    dyn Fn(
            Option<u64>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Vec<crate::symbio_core::PromptMessage>> + Send>,
        > + Send
        + Sync,
>;

impl std::fmt::Debug for TurnInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnInput")
            .field("turn", &self.turn)
            .field("text", &self.text)
            // 中止信号只打印「有没有」：`ExecAbortSignal` 是内部句柄（AtomicBool +
            // 通知），展开它对本入参包的可读性没有增量，反而多出一堆内部字段。
            .field("abort", &self.abort.as_ref().map(|_| "<信号>"))
            .field("tier", &self.tier)
            .field("window_turns", &self.window_turns)
            .field("resume", &self.resume)
            .field("actor", &self.actor)
            .field("prefix", &self.prefix)
            .field("stages", &self.stages)
            .field("final_reply", &self.final_reply)
            .field("dialog", &self.dialog)
            .field("opening_text", &self.opening_text)
            .field("opening_dialog", &self.opening_dialog)
            .field("turn_root_id", &self.turn_root_id)
            .field("user_node_id", &self.user_node_id)
            .field("user_meta", &self.user_meta)
            // 闭包**不展开**：展开会把捕获的东西（生产里是用户对话正文）打进日志。
            .field("inject", &self.inject.as_ref().map(|_| "<闭包>"))
            .field("nodes", &self.nodes.as_ref().map(|_| "<闭包>"))
            .finish()
    }
}

/// 续写锚点（审批 / 问答恢复）：本轮**续写**一个已开未收束的轮次，而不是新开。
///
/// ## 为什么必须续写同一轮，而不是新开一轮
///
/// C4（`symbio_core::unresolved_turns`，core 的 `invariants` 域）按 **turn 号**配对：
/// 一个 `user.message` 只有遇到**同号**的收束事件才算收束。等待用户的那一轮已经
/// 落了用户格、没落收束格（`awaits_user` 是「还没完」的诚实缺口）；若恢复时另开
/// 新轮，原轮永远等不到收束事件，C4 会把它当**永久缺口**（假阳性）——不变量随即
/// 失去判据价值。续写同一轮才是它的解：恢复后收束仍记在该轮上，缺口被真正填上。
///
/// ## 为什么用户格不重开
///
/// 恢复请求（approve / reject / answer）**没有**新的用户发言（`single_message`
/// 为 `None`）；重开一格会让同一句话在网格里出现两次，且视图窗口（本模块的
/// `window_by_turn`）会把那一轮数成两轮。
#[derive(Debug, Clone)]
pub struct TurnResume {
    /// 被续写轮次的用户格 `seq`（`produced_by` 的锚点，I2）。
    pub user_seq: u64,
    /// 恢复后的工具调用（渲染 prompt 的交换段；形状复用
    /// [`crate::symbio_core::TurnToolCallInfo`]，
    /// 不另立一份——`name` / `arguments` 的取法必须与在途工具轮逐字一致）。
    pub call: crate::symbio_core::TurnToolCallInfo,
    /// 恢复后的结果正文（落 `artifact.added`，并作为交换段的结果行）。
    pub text: String,
    /// **被恢复的 ToolCall 节点 id**（恢复请求的 `target_id`）。
    ///
    /// 续写轮落 `artifact.added` 时记进载荷的 `tool_call_id`：读路径走 WAL 重建后，
    /// 「结果挂在哪个调用之下」只能靠这条事实复刻。空缺（老调用点 / 单测）时回落到
    /// 按本轮 `turn_root_id` 合成——与接线前同形，但那会让原调用成孤儿（e2e t27）。
    pub call_id: Option<String>,
    /// **被恢复轮次的 Turn 根节点 id**（原调用节点的 `parent_id`）。
    ///
    /// 与 [`Self::call_id`] 同批入格：重建把调用节点落在这一轮上，缺了就长出新 Turn 根。
    pub turn_id: Option<String>,
    /// 恢复方分配的**结果子节点 id**（`resume.rs` 步 7）。
    ///
    /// `Carrier::Runner` = 这条节点没有出生事实，承载它的是续写轮的 `artifact.added`；
    /// 载荷复用同一个 id，实时面与重建面才是同一条节点。
    pub result_id: Option<String>,
}

/// 一轮的结果：`fell_back = false` ⇒ 模型作答；`true` ⇒ 兜底（`text` 即
/// 失败原因，可观测——I3 的「到点必答」落在这里）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub turn: u64,
    pub text: String,
    pub cost_ms: u64,
    /// 本轮**最后一次**模型响应的实测用量（provider 没给就是 `None`）。
    ///
    /// 为什么只带最后一次：[`Self::text`] / 推理正文都是最后一轮的产物，校准比
    /// 的分子分母必须**同一次响应**（`chat_loop/turn.rs` 的 `feedback_estimate`）
    /// ——拿多轮的合计用量对最后一轮的文本做分母，观测比会系统性偏高。
    /// 反射档 / 兜底 / 中止没有模型参与，恒 `None`。
    pub usage: Option<crate::symbio_core::ModelUsage>,
    pub fell_back: bool,
    /// 中止（用户主动停止 / 会话销毁）：**中止照落收束格**（`chat.assistant.final`，
    /// 载荷 `status: "aborted"` + `terminated: { reason, at }`）——模型已产出的半截
    /// 正文 / 半截思考是事实，`text` 就是那半截正文（可为空）。收束 kind 绝不复用
    /// 兜底格（`chat.assistant.fallback` 是**失败**的形状），因此与 `fell_back` 互斥、
    /// **绝不进兜底分子**（兜底率是失败的指标：用户按的停止不是「模型答不出」）。
    pub aborted: bool,
    /// 本轮**收束于等待用户动作**（工具报了 `failure_kind = pending`，如 confirm /
    /// ask_user）：运行器停止工具循环并落**进展格**（`chat.assistant.progress`，
    /// `turn × progressed`，载荷 `status: "pending"`）——本轮尚未了结，模型说过的话
    /// 照样是事实；恢复是 [plan/11 批 2](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") 的
    /// 能力，恢复后以收束格了结（进展格不占「一轮至多一条收束格」）。
    pub awaits_user: bool,
    /// 本轮收束**被写侧闸拒绝**（主体不持 `reply.first`，见 [`closure_granted`]）：
    /// 收束格未入格、且本轮**没有落过产物格**，该轮留在未收束态（`check_all` 的 C4
    /// 报得出）。与 `aborted` / `awaits_user` 同属「网格少一格」，但**原因在授权**，
    /// 不是放弃也不是等待——调用方据此**跳过本轮的派生事实**（承诺 / 任务 / 熔断 /
    /// 记忆全不写）。
    ///
    /// **落过产物格 ⇒ 不算拒**（[ADR-067](../../../../docs/010-adr/067.md "ADR-067")）：
    /// 产物收束是另一条收束路（`ClosureKind::Artifact`），它落过格就说明这一轮有话
    /// 说过——典型是只持 `produce.artifact` 的对等主体：发言闸拒它，但它产出的
    /// 工具结果是事实，派生事实照写。
    ///
    /// 平凡值 `false`：生产里主体恒为登记过的行（主智能体一族），持 `reply.*` ⇒ 闸是
    /// fail-closed 的**结构**，不是会翻面的开关。未登记主体没有行（[ADR-099]）⇒
    /// 闸拒、此位为真——那是超权形态被收回来的信号，不是缺省值。
    pub closure_denied: bool,
    /// 本轮**最后一次**模型响应的结束原因（provider 归一化，见 [`ModelFinishReason`]）。
    ///
    /// 与 [`Self::usage`] 同一条口径：都是**最后一次**响应的事实（多轮工具循环里
    /// 前面几轮的原因随 `rounds[]` 入格，不进这个字段）。「本轮是不是被 `max_tokens`
    /// 掐断」只能看末轮。反射档 / 兜底 / 中止没有末轮 ⇒ 恒 `Stop`（缺省）。
    ///
    /// 唯一消费方是 [`super::chat_loop::settle_reasoning`]：`is_length()` 打长度戳，
    /// [`super::chat_loop::close_turn`] 在同判据上触发自动续写（`MAX_CONTINUE_ROUNDS`）。
    /// 这一格**不得**经 `TurnOutput { ..Default::default() }` 重建：那会恒填 Stop，
    /// 长度截断与自动续写因此不可达（T-144）。
    pub finish: crate::symbio_core::ModelFinishReason,
}

/// turn 运行器：持令牌的调用方对每个会话轮调用一次。
pub struct TurnRunner;

impl TurnRunner {
    /// 带工具的流式轮：**工具清单下行、工具调用上行、分发、产物落格**，直到模型
    /// 不再请求工具为止。
    ///
    /// ## 网格记账
    ///
    /// - 用户消息：`turn × opened`（`user.message`），一次；
    /// - 每轮工具调用：**不单独占格**——工具结果才是产物事实，落
    ///   `artifact × asserted`（`artifact.added`，载荷 `{ tool, text }`），
    ///   `produced_by` 指向**本轮用户格**（S02 §3 的 `produced_by` 断言）；
    /// - 收束：`chat.assistant.final`（或失败的兜底格）——**只落一次**。
    ///
    /// ## prompt 的两段
    ///
    /// 基线由 [`crate::symbio_core::actors::render_messages`] 从转写投影出（历史来自事实源，含**往轮**的
    /// 工具结果——[plan/10 批 3](../../../../docs/004-plan/008-工具轮v2化实施方案.md "PL-008-工具轮v2化实施方案") 起）；
    /// 而**本轮**刚发生的交换，投影里当然还没有（快照取在它落格之前），故由本函数
    /// **就地累积**追加——模型必须看到自己请求过什么、拿到了什么，否则会重复调用
    /// 同一个工具。
    ///
    /// ## 中止 / 等待用户
    ///
    /// 两者都照落一格事实——**思考 / 正文 / 工具是事实，完成 / 终止 / 等待是状态**：
    /// 中止落收束格（`chat.assistant.final`，载荷 `status: "aborted"` +
    /// `terminated: { reason, at }`，半截正文 / 半截思考一并入格），绝不复用兜底格
    /// （那是**失败**的形状，且会污染兜底率）；等待用户落进展格
    /// （`chat.assistant.progress`，`turn × progressed`，载荷 `status: "pending"`），
    /// 不收束轮次（[plan/11 批 2](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
    /// 恢复后续写同一轮、以收束格了结）。「一轮至多一条收束格」不变。
    ///
    /// ## 为什么参数多到要 `allow`
    ///
    /// 七个参数**各自是一个不同的端口**（事实源 / 生成 / 令牌 / 输入 / 流式出口 /
    /// 工具清单 / 分发通道），没有两个属于同一概念——打包成一个结构体只是把
    /// 「七个端口」改名叫「一个结构体 + 七个字段」，调用方仍要逐个填，
    /// 却多出一层只为过 lint 而生的壳。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_with_tools<S>(
        &self,
        store: &S,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        input: TurnInput,
        sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
        tools: &[crate::symbio_core::CapabilityMeta],
        dispatch: Option<&dyn crate::symbio_core::DispatchPort>,
    ) -> Result<TurnOutcome, crate::symbio_core::AppendError>
    where
        // `?Sized`：调用方持的是 `&dyn Store`（loop 只见 trait object，不见会话存储实现）。
        S: Store<Event = Event> + ?Sized,
    {
        let mut run = match prepare::prepare_turn(store, llm, input, sink.clone()).await? {
            prepare::PreparedFlow::Done(outcome) => return Ok(outcome),
            prepare::PreparedFlow::Run(run) => run,
        };
        loop {
            if let Some(outcome) =
                stream::stream_round(&mut run, store, llm, tok, sink.clone(), tools, dispatch)
                    .await?
            {
                return Ok(outcome);
            }
        }
    }
}

/// 收束格的**种类**（[ADR-065] 裁决、[ADR-067] 落地形态）：同一道写侧闸按种类分派。
///
/// 一轮的收束有两条路，谓词各问自己那一项能力：
///
/// | 种类 | 落什么格 | 谓词 |
/// |---|---|---|
/// | [`ClosureKind::Speech`] | `chat.assistant.final` / `.fallback` | `can_reply`（`reply.first`） |
/// | [`ClosureKind::Artifact`] | `artifact.added` | `can_produce_artifact`（`produce.artifact`） |
///
/// 为什么必须分种类：对等主体（`agent:peer-b`，授权表只给它 `produce.artifact`）
/// 在发言闸下**恒拒**——合并前它连一轮都收束不了，「对等 offer / assert」的写方
/// （住轮末收束族）因此永远执行不到。分种类后它能收束为**一件产物**，而它的发言
/// 资格仍然一项都没有（「没有发言权」与「留不下任何事实」是两件事）。
///
/// [ADR-065]: ../../../../docs/010-adr/065.md "ADR-065"
/// [ADR-067]: ../../../../docs/010-adr/067.md "ADR-067"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClosureKind {
    /// 发言收束：答话 / 兜底 / 中止的那一格。
    Speech,
    /// 产物收束：工具结果的那一格（`artifact.added`）。
    Artifact,
}

impl ClosureKind {
    /// 种类名——`authz.denied` 载荷的 `kind` 与幂等键的尾段都用它。
    ///
    /// 一个名字一个来源：载荷里写一套、幂等键里写另一套，读侧按 `kind` 分组时
    /// 会漏掉一半（两套都是合法拼写，谁也不报错）。
    fn name(self) -> &'static str {
        match self {
            ClosureKind::Speech => "speech",
            ClosureKind::Artifact => "artifact",
        }
    }

    /// 这一类格要的**能力名**（拒绝原因里说的就是它）。
    ///
    /// 拼写与 `Capability::Reply` / `Capability::ProduceArtifact` 的 `.name()` 同值，
    /// 但本模块看不见 `governance`（core 的私有子模块）——所以这里写一遍，
    /// 由 `denial_names_the_capability_the_gate_judges` 用 `ActorSpec::holds`
    /// **对拍**：拼错一个字符，那条用例当场红。手写一份没有对拍，就只是又一处
    /// 「表授予 A、闸门判 B」。
    fn capability_name(self) -> &'static str {
        match self {
            ClosureKind::Speech => "reply",
            ClosureKind::Artifact => "produce.artifact",
        }
    }
}

/// 写侧授权闸（[plan/01 §7](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构") 写侧，
/// [04 §3.1 批⑥](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）：该主体此刻能否写
/// `kind` 所指的那一格。
///
/// ## 为什么判在运行器
///
/// 收束格与产物格都由**本运行器**原生落格，闸就在落格**之前**判。三条发言路径
/// （深度档 final / 反射档 final / 兜底格）与两条产物路径（续写轮 / 工具结果）
/// 都过这里——同一种类的格是**同一个事实**，不论它由哪条路径写出。
///
/// ## 谓词与判据
///
/// 谓词是 `PermissionMatrix` 的两种判定（写侧闸判什么**只定义一次**，见
/// `symbio_core::governance`）：`Speech` 问 `can_reply`，`Artifact` 问
/// `can_produce_artifact`。`Speech` 的 `first` 恒为 `true`：运行器一轮只落一格
/// 发言收束（`final_unique_per_turn` 是不变量），不存在「同轮追加」那一态。
///
/// 拒绝 ⇒ **不落格，但落一条否定事实**（`authz.denied`，见 [`deny`]）：
/// - 发言臂 ⇒ 该轮留在未收束态，`check_all` 会把它报出来（C4）——**除非本轮落过
///   产物格**，那同样算收束（`unresolved_turns` 与产物闸同批认可）；
/// - 产物臂 ⇒ 那一条工具结果不入格（warn 后跳过）。
///
/// 「不落格」与「落否定事实」不矛盾：被拒的那一格确实没写（那是权限的结论），
/// 而「这次拒绝发生过」本身是事实（那是 I1 的要求）。两者缺一：只不落格 ⇒
/// 「被拒」与「没跑完」在日志上同形；只落格 ⇒ 权限形同虚设。
///
/// ## 第二道：作用域上限（[T-105]）
///
/// 主体自己那一行之外还有一道——**委派不放大权威**：`scope = child <parent>` 的
/// 主体（`agent_run` 派生的子会话）能落的格，其**父主体也必须能落**。
/// `root` = 独立主体，无上限。语义与取值点见
/// [T-128 §3.2](../../../../docs/009-task/128-多主体与对等承诺.md "T-128-多主体与对等承诺")
/// 与 `chat_loop::state::request_scope`。
///
/// 平凡值下（顶层会话 = `root`，以及主智能体派生的子会话）两道都是恒真——
/// 闸是 fail-closed 的**结构**，不是生产里会翻面的开关。
///
/// [T-105]: ../../../../docs/009-task/105-Scope-Child首产点批.md "T-105-Scope-Child首产点批"
fn closure_granted(actor: &ActorSpec, kind: ClosureKind) -> bool {
    closure_gate(actor, kind).is_ok()
}

/// 与 [`closure_granted`] **同一个判定**，但把「为什么拒」带出来。
///
/// 分成两个函数而不是各写一份谓词：`closure_granted` 是 `is_ok()` 的一行壳，
/// 判据只有这一处。两处各写一套「这一类格要什么能力」就会让闸门与拒绝原因分叉
/// ——那时 `denied` 事实记的原因与实际拒它的理由不是同一件事，比不记更坏。
///
/// 原因字符串里带**能力名**（`Capability::name()`）而不是「缺 reply.first」这种
/// 手写措辞：能力名是表里的同一份数据，改了名字这里跟着改；手写措辞不会。
fn closure_gate(actor: &ActorSpec, kind: ClosureKind) -> Result<(), String> {
    // 八个收束调用点全在「本轮尚无收束格」的位置，因此发言只问一个 `Capability::Reply`
    // ——名住 core，规格方法与表谓词共用它（ADR-103：不按轮次位置分能力）。
    let own = match kind {
        ClosureKind::Speech => actor.can_land_speech(),
        ClosureKind::Artifact => actor.can_land_artifact(),
    };
    if !own {
        return Err(format!(
            "{} 不持 {}",
            actor.principal,
            kind.capability_name()
        ));
    }
    within_delegation_ceiling(actor, kind)
}

/// 落一条**否定事实**（[`EVENT_AUTHZ_DENIED`]，T-219）。
///
/// 闸门拒绝时此前只留一条 `warn`：日志里剩下的是「该轮没收束」，与「根本没跑完」
/// 同形。本函数把否定落成事实——判定与矩阵**一个字不动**，变的只是否定的表达。
///
/// ## 为什么是「落事实」而不是「返回错误」
///
/// I1 单通道：一切状态变更只经事实源。「被拒绝」是一个状态变更，因此它也必须
/// 是一条事实——否则不可审计（谁 / 何时 / 因何被拒）、不可回放（没有东西可重放）。
///
/// ## 三条边界（与 [`EVENT_AUTHZ_DENIED`] 的载荷说明同源）
///
/// 1. **不是收束格**：落在 `turn × asserted`。该轮仍然未收束，`unresolved_turns`
///    照报——把它算成收束等于把「静默中断」这条最该被发现的失效一起掩盖掉。
/// 2. **幂等**：键 `deny-{turn}-{kind}`，`Duplicate` 静默吞掉。产物臂在工具结果的
///    循环里，多轮 / 多个结果会重复走到闸前，而它们说的是同一件事。
/// 3. **记账失败不改变结论**：落不进去也照原样返回拒绝结果（warn 里带错误）。
///    闸门的结论是「不许写」，不能因为记账失败就放行。
///
/// `key` = 被拒的**目标格名**（`chat.assistant.final` / `.fallback` / `artifact.added`）。
/// 义务环落地后它会换成 `Pattern` 的键（PL-016 §4.2 的 `denied(key)`）——今天没有
/// `Decl`，先用格名，那是同一件事在日志里的名字。
fn deny<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    kind: ClosureKind,
    key: &str,
    why: &str,
) where
    S: Store<Event = Event> + ?Sized,
{
    let kind_name = kind.name();
    let event = Event::pending(
        format!("deny-{turn}-{kind_name}"),
        EVENT_AUTHZ_DENIED,
        Entity::Turn,
        Verb::Asserted,
        turn,
        &actor.principal,
    )
    .with_produced_by(user_seq)
    .with_payload(serde_json::json!({
        "key": key,
        "kind": kind_name,
        "why": why,
    }));
    match store.append(event) {
        Ok(_) => {}
        // 同一轮同一格被拒多次：说的是同一件事，落一条就够。
        Err(crate::symbio_core::AppendError::Duplicate) => {}
        Err(e) => crate::plugin_warn!(
            "actors",
            "[v2] 拒绝事实未入格（turn={turn} / {kind_name} / {key}）：{e:?}"
        ),
    }
}

/// 作用域上限（[T-105]）：`scope = child <parent>` ⇒ 父主体也必须能落这一类格。
///
/// 谓词与 [`closure_granted`] 的**同一份**（`can_reply` / `can_produce_artifact`）
/// ——两处各写一套「这一类格要什么能力」就会让上限与许可分叉。
fn within_delegation_ceiling(actor: &ActorSpec, kind: ClosureKind) -> Result<(), String> {
    let crate::symbio_core::Scope::Child(parent) = &actor.scope else {
        return Ok(());
    };
    let matrix = crate::symbio_core::authz::production_matrix();
    let granted = match kind {
        ClosureKind::Speech => matrix.can_reply(parent),
        ClosureKind::Artifact => matrix.can_produce_artifact(parent),
    };
    if granted {
        return Ok(());
    }
    Err(format!(
        "父主体 {parent} 不持 {}（委派不放大权威）",
        kind.capability_name()
    ))
}

#[cfg(test)]
#[path = "turn_runner.test.rs"]
mod tests;
