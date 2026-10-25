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
//! `../../../../docs/003-design/001-核心架构.md` = 0。真正的冻结锚点
//! （[plan/03 §1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）是：F1 `Store` 签名 /
//! F2 `Projection` 构造形状 / **F3 `ActorSpec` 字段集 + 档位→令牌映射** /
//! F4 三条不变量 / F5 三个封闭集合 / F6 参数表键集。**`TurnRunner` 不在其中**，
//! 所以这次下沉**不改 F1–F6**，按 [plan/03 §2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")
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
    EVENT_SESSION_CONSOLIDATED, EVENT_TURN_DISPATCH,
};
use std::borrow::Cow;

// ── 阶段拆分层（· B3，DE-021 §5 动作 B）──────────────────────────
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
pub(crate) use self::prepare::{PendingClose, Prepared};

// ── v2 会话运行时：turn 运行器（chat_loop 切换的第一块可复用件）────────
//
// 职责把 [plan/04 §2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") 的单轮流程落成
// 一个函数调用：用户消息入格 → [`Reasoner`] 生成（实测耗时）→ final / fallback
// 落格（I3：失败也必须有输出）。不变量靠构造成立：每轮独立 turn 号（N3）、
// final 溯源指向本轮用户消息（N5）、实测 cost_ms 随事件入账（D-044）。
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
    /// 唤醒幕（审批 / 问答恢复）填的是**被续写轮**的号（已开未收束的那一轮），
    /// 不是新号。
    pub turn: u64,
    /// 本轮**用户格的 seq**——判决的溯源锚点（I2 / ）。
    ///
    /// 用户格已由**调用方**在进运行器之前落格（D-114：事实先于任何判决），
    /// 因此这里收到的是它的位置而不是正文：运行器不再拥有「用户说了什么」，
    /// 只拥有「拿哪一格当锚」。续写轮同样填被续写轮的用户格。
    pub user_seq: u64,
    /// 本次执行的**幕号**（`turn.dispatch` / 阶段事实的幂等键尾段，）。
    ///
    /// 新开一轮 = 用户格的 seq；续写幕 = 续写阶段事实的 seq。同一轮里多次执行
    /// （恢复后的续写幕）因此各有一组互不撞号的格。
    pub drive_seq: u64,
    /// **本轮的中止信号**（D-052：中止是事实的**状态**，不是「什么都没发生」）。
    ///
    /// ## 为什么运行器要拿它
    ///
    /// 中止可以在**用户格落完之后、第一次 LLM 请求之前**到达（用户按下停止的
    /// 那一刻恰好落在闸门与请求之间）。若拦在进运行器**之前**（`TurnExit::Aborted`），
    /// 运行器就从不落 `user.message` 格：WAL 全空，中止这一轮**一条事实都没有**
    /// （e2e t7 钉的是这件事：`v2-events.wal` 不存在，恢复后重建的历史里看不到
    /// 这轮说过什么）。而 D-052 决策 1 要求「一轮的产出无论完成、失败、被中断
    /// 还是悬在等待用户，都入格」。
    ///
    /// 形状 = 检查点在运行器内：落完用户格后、发第一个请求前检查一次，
    /// 已中止则直接走**中止收束格**（`chat.assistant.final` + `status: "aborted"` +
    /// `terminated`）——与 adapter 中途返回 `AdapterError::Aborted` 落的是同一格。
    ///
    /// `None` = 调用方不提供信号（单测 / 无中止语义的路径），行为与接线前逐字一致。
    pub abort: Option<crate::symbio_core::ExecAbortSignal>,
    /// 本轮装配进哪一时延档（调度决定是数据，D-044）。
    pub tier: LatencyTier,
    /// 对话窗口（最近的 turn 数，**含当前轮**）；`None` = 全量。
    ///
    /// 会话轮必填——WAL 里的事实只增不减，不带窗口的 prompt 会随会话
    /// 无界增长；窗口值由调用方的窗口配置给出（如 `context_messages`）。
    pub window_turns: Option<u64>,
    /// **本轮以哪个主体的身份记账**——收束 / 产物事件的 `actor`、窗口可见域的
    /// `viewer` 都取它（plan/11 批 1
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
    /// 生成 [`Self::final_reply`] 的那次措辞往返的**实测耗时**（毫秒；模板 / 无模型
    /// 往返的产线为 `0`）。
    ///
    /// 定稿答话轮不进驱动循环，本轮唯一的模型往返发生在 `compose/wording` 里。
    /// 收束格由运行器就地落，若这里恒记 `0`，`cost_ledger` 与 SLO 的台账会把
    /// 整类 `Answered` 轮次的模型成本漏掉（成本台账的唯一输入仍是落格事件，
    /// 但这次往返的代价属于这一轮的收束格）。取值链：
    /// `ProviderLlmAdapter::generate_turn` 实测 → `WordingReply.cost_ms` →
    /// 此处 → `chat.assistant.final.cost_ms`。
    pub final_cost_ms: u64,
    /// 定稿答话**对话面节点**的身份与标记（仅 `final_reply` 有值时消费）。
    ///
    /// ## 为什么答话要带这三样，而不是只有正文
    ///
    /// `Answered` 的答话是一条**根级**对话面节点（`parent_id = None`，带
    /// `meta.surface = "reply"` / `meta.reason` / 可选的 `meta.exclude_from_context`）。
    /// 收束格若只记正文（`final_reply`），重建便**折不出这条节点**——实测
    /// t20 / t21 / t22 / t25 全部「0 条对话面节点」。节点形状是事实的一部分
    /// （原则），故随收束格一并入格（WAL 为唯一事实源，D-049）。
    pub dialog: Option<DialogFacts>,
    /// **对话面开场白**：`Escalate` 首响正文（`Some` = 本轮有一句开场白）。
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
            .field("user_seq", &self.user_seq)
            .field("drive_seq", &self.drive_seq)
            // 中止信号只打印「有没有」：`ExecAbortSignal` 是内部句柄（AtomicBool +
            // 通知），展开它对本入参包的可读性没有增量，反而多出一堆内部字段。
            .field("abort", &self.abort.as_ref().map(|_| "<信号>"))
            .field("tier", &self.tier)
            .field("window_turns", &self.window_turns)
            .field("actor", &self.actor)
            .field("prefix", &self.prefix)
            .field("final_reply", &self.final_reply)
            .field("final_cost_ms", &self.final_cost_ms)
            .field("dialog", &self.dialog)
            .field("opening_text", &self.opening_text)
            .field("opening_dialog", &self.opening_dialog)
            .field("turn_root_id", &self.turn_root_id)
            // 闭包**不展开**：展开会把捕获的东西（生产里是用户对话正文）打进日志。
            .field("inject", &self.inject.as_ref().map(|_| "<闭包>"))
            .field("nodes", &self.nodes.as_ref().map(|_| "<闭包>"))
            .finish()
    }
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
    /// 载荷 `status: "aborted"` + `terminated: { at }` + 顶层 `reason: interrupted`）——模型已产出的半截
    /// 正文 / 半截思考是事实，`text` 就是那半截正文（可为空）。收束 kind 绝不复用
    /// 兜底格（`chat.assistant.fallback` 是**失败**的形状），因此与 `fell_back` 互斥、
    /// **绝不进兜底分子**（兜底率是失败的指标：用户按的停止不是「模型答不出」）。
    pub aborted: bool,
    /// 本轮**收束于等待用户动作**（工具报了 `failure_kind = pending`，如 confirm /
    /// ask_user）：运行器停止工具循环并落**进展格**（`chat.assistant.progress`，
    /// `turn × progressed`，载荷 `status: "pending"`）——本轮尚未了结，模型说过的话
    /// 照样是事实；恢复是 plan/11 批 2 的
    /// 能力，恢复后以收束格了结（进展格不占「一轮至多一条收束格」）。
    pub awaits_user: bool,
    /// 本轮收束**被写侧闸拒绝**（主体不持 `reply.first`，见 [`closure_gate`]）：
    /// 收束格未入格、且本轮**没有落过产物格**，该轮留在未收束态（`check_all` 的 C4
    /// 报得出）。与 `aborted` / `awaits_user` 同属「网格少一格」，但**原因在授权**，
    /// 不是放弃也不是等待——调用方据此**跳过本轮的派生事实**（承诺 / 任务 / 熔断 /
    /// 记忆全不写）。
    ///
    /// **落过产物格 ⇒ 不算拒**（[D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "DE-011-Agent目录规范")）：
    /// 产物收束是另一条收束路（`ClosureKind::Artifact`），它落过格就说明这一轮有话
    /// 说过——典型是只持 `produce.artifact` 的对等主体：发言闸拒它，但它产出的
    /// 工具结果是事实，派生事实照写。
    ///
    /// 平凡值 `false`：生产里主体恒为登记过的行（主智能体一族），持 `reply.*` ⇒ 闸是
    /// fail-closed 的**结构**，不是会翻面的开关。未登记主体没有行（[D-099]）⇒
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
    /// 长度截断与自动续写因此不可达。
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
    /// 工具结果——plan/10 批 3 起）；
    /// 而**本轮**刚发生的交换，投影里当然还没有（快照取在它落格之前），故由本函数
    /// **就地累积**追加——模型必须看到自己请求过什么、拿到了什么，否则会重复调用
    /// 同一个工具。
    ///
    /// ## 中止 / 等待用户
    ///
    /// 两者都照落一格事实——**思考 / 正文 / 工具是事实，完成 / 终止 / 等待是状态**：
    /// 中止落收束格（`chat.assistant.final`，载荷 `status: "aborted"` +
    /// `terminated: { at }` + 顶层 `reason: interrupted`，半截正文 / 半截思考一并入格），绝不复用兜底格
    /// （那是**失败**的形状，且会污染兜底率）；等待用户落进展格
    /// （`chat.assistant.progress`，`turn × progressed`，载荷 `status: "pending"`），
    /// 不收束轮次（plan/11 批 2
    /// 恢复后续写同一轮、以收束格了结）。「一轮至多一条收束格」不变。
    ///
    /// ## 为什么参数多到要 `allow`
    ///
    /// 七个参数**各自是一个不同的端口**（事实源 / 生成 / 令牌 / 输入 / 流式出口 /
    /// 工具清单 / 分发通道），没有两个属于同一概念——打包成一个结构体只是把
    /// 「七个端口」改名叫「一个结构体 + 七个字段」，调用方仍要逐个填，
    /// 却多出一层只为过 lint 而生的壳。
    /// 起一轮：准备阶段可能直接收束（反射档 / 早退），否则交回一个
    /// [`TurnSession`]。**这里不再有循环**——步的推进归调用方。
    pub(crate) async fn begin<S>(
        store: &S,
        llm: &dyn LlmAdapter,
        input: TurnInput,
        sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
    ) -> Result<TurnSessionBegin, crate::symbio_core::AppendError>
    where
        S: Store<Event = Event> + ?Sized,
    {
        match prepare::prepare_turn(store, llm, input, sink).await? {
            prepare::PreparedFlow::Done(outcome) => Ok(TurnSessionBegin::Done(outcome)),
            prepare::PreparedFlow::Run(run) => {
                Ok(TurnSessionBegin::Run(TurnSession { run, steps: 0 }))
            }
        }
    }
}

/// 开一轮的结果：`Done` = 本轮已收束，`Run` = 还有动作要跑。
pub(crate) enum TurnSessionBegin {
    Done(TurnOutcome),
    Run(TurnSession),
}

/// 一轮的**会话草稿**：持有跨动作状态，但**不持有驱动**。
///
/// 驱动归派发器（`turn_exec::drive_steps`），「这两个动作跑哪一个」归执行体
/// （[`super::turn_exec::run_execute`] 读草稿决定）；本类型只回答「这一个动作
/// 发生了什么」（D-118 决策 4）。动作有五个，各自只写自己那一半事实：
///
/// - [`Self::model`]：一次模型往返——拼消息、调模型、分类；请求工具时把应答留在
///   草稿里，不派发；该收束时把**结局**留在 [`prepare::Prepared::closing`]，不收束；
/// - [`Self::open`]：为模型交回的调用逐条开账（`open-tools` 行的执行体）；
/// - [`Self::authorize`]：对已开账的调用判放行 / 拒绝（`authorize-tool` 行）；
/// - [`Self::dispatch`]：派发那一批工具，落产物 / 等待 / 交换（`dispatch-tools` /
///   `deny-tools` 两行）；
/// - [`Self::close`]：落寄存的那份结局（`close-turn` 行的执行体）。
///
/// 停机判据（步数上界 / 无进展 / 中止 / 租约）不在这里，归派发器（D-118 决策 7）；
/// 本类型的 [`Self::exhausted`] 只是那三格收束事实的**唯一写方**。
pub(crate) struct TurnSession {
    run: Box<prepare::Prepared>,
    steps: usize,
}

impl TurnSession {
    /// 记一次模型往返（执行体自己知道这一步有没有调模型）。
    ///
    /// 只数**模型往返**：工具批次不额外消耗步数，与拆分前「一步 = 一轮往返」同口径。
    /// 上界判断不在这里——停机判据归派发器（D-118 决策 7），它读
    /// [`Self::over_step_limit`]。
    pub(crate) fn record_model_step(&mut self) {
        self.steps += 1;
    }

    /// 模型往返步数是否已到上界（派发器每步前问一次，到顶即 `exhausted{step_limit}`）。
    pub(crate) fn over_step_limit(&self) -> bool {
        self.steps >= MAX_STEPS_PER_DRIVE
    }

    /// 跑**一次模型往返**：请求了工具就把应答留在草稿里等 [`Self::tools`]；
    /// 该收束了（答完了 / 模型没答成 / 中止）就把**结局**留给 `close-turn`。
    ///
    /// 只有一条路就地收口：**控制面**失败（模型往返压根没派发）——它在日志上没有
    /// 模型的终局事实，收束行开不出账，兜底格只能由这一步落（`execute` 执行体的
    /// 过渡豁免，见 `loop_rows::ACTORS`）。
    ///
    /// 它**不交回**产出（D-123 决策 5）：「本轮结束了没有」由派发器读账本判，
    /// 执行体只写自己那一半事实。
    pub(crate) async fn model<S>(
        &mut self,
        store: &S,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
        tools: &[crate::symbio_core::CapabilityMeta],
        has_dispatch: bool,
    ) -> Result<(), crate::symbio_core::AppendError>
    where
        S: Store<Event = Event> + ?Sized,
    {
        match stream::model_round(&mut self.run, store, llm, tok, sink, tools, has_dispatch).await?
        {
            stream::ModelRound::Done(outcome) => self.run.settled = Some(outcome),
            stream::ModelRound::Tools => {}
            // 应答已寄存，收束这一步归 `close-turn` 行的执行体（[`Self::close`]）。
            stream::ModelRound::Close => {}
        }
        Ok(())
    }

    /// 落寄存的那份**结局**：答完了落 `chat.assistant.final`、模型没答成落
    /// `chat.assistant.fallback`、中止也落 `chat.assistant.final`（中止是**状态**，
    /// 不是另一种事实）；产出寄存在草稿。
    ///
    /// 它是 `close-turn` 行的动作（/ 步 5c）。跑不跑这一步由**日志**
    /// 决定（`turn.closeable`），不由这里决定；没有寄存时它不做任何事，派发器的
    /// 「无进展」判据会把那种收错账的形态显形。
    pub(crate) async fn close<S>(
        &mut self,
        store: &S,
        llm: &dyn LlmAdapter,
    ) -> Result<(), crate::symbio_core::AppendError>
    where
        S: Store<Event = Event> + ?Sized,
    {
        self.run.settled = stream::close_round(&mut self.run, store, llm).await?;
        Ok(())
    }

    /// 创建动作（`open-tools` 行的执行体）：开账，结果寄存在草稿里（b）。
    ///
    /// 没有模型应答 / 没有分发通道时不做任何事——派发器只在 `open-tools` 欠时才选
    /// 这个动作，这里的空返回是防御，不是第二条选择路径。
    pub(crate) async fn open(
        &mut self,
        store: &dyn Store<Event = Event>,
        dispatch: Option<&dyn super::tool_dispatch::ToolDispatch>,
    ) {
        if let Some(dispatch) = dispatch {
            stream::open_round(&mut self.run, store, dispatch).await;
        }
    }

    /// 授权动作（`authorize-tool` 行的执行体）：对已开账的调用落放行 / 拒绝。
    pub(crate) async fn authorize(
        &mut self,
        store: &dyn Store<Event = Event>,
        dispatch: Option<&dyn super::tool_dispatch::ToolDispatch>,
    ) {
        if let Some(dispatch) = dispatch {
            stream::authorize_round(&mut self.run, store, dispatch).await;
        }
    }

    /// 派发动作（`dispatch-tools` / `deny-tools` 两行的执行体）：落 `sent`、跑执行
    /// 层、关账；继续下一轮就把交换累加进草稿，收束于等待用户就把产出寄存在草稿。
    ///
    /// 没有草稿 / 没有分发通道时不做任何事——同上，防御而不是选择路径。
    pub(crate) async fn dispatch(
        &mut self,
        store: &dyn Store<Event = Event>,
        llm: &dyn LlmAdapter,
        dispatch: Option<&dyn super::tool_dispatch::ToolDispatch>,
    ) -> Result<(), crate::symbio_core::AppendError> {
        if let Some(dispatch) = dispatch {
            self.run.settled = stream::dispatch_round(&mut self.run, store, llm, dispatch).await?;
        }
        Ok(())
    }

    /// 派发器收工时取走本轮产出（[`prepare::Prepared::settled`]）。
    ///
    /// 取（`take`）而不是看：一次 drive 只交回一个产出，留着的第二次读会拿到上一轮的。
    pub(crate) fn take_settled(&mut self) -> Option<TurnOutcome> {
        self.run.settled.take()
    }

    /// 本动作属于哪一轮（派发器读事实切片与日志用）。
    pub(crate) fn turn(&self) -> u64 {
        self.run.turn
    }

    /// 本次 drive 的 `now`（幕锚那条唤醒事实的 `ts`，判据 12）——派发器选债用它。
    pub(crate) fn now_ms(&self) -> i64 {
        self.run.now_ms()
    }

    /// 停机收束：落兜底格且 `reason = exhausted`（`step_limit` / `no_progress`）。
    ///
    /// 派发器在「表里没有可还的债」或「执行体返回空且事实没长」时调用它；
    /// 收束判据仍只有这一处写方（D-115 决策 3–4）。
    pub(crate) async fn exhausted<S>(
        &mut self,
        store: &S,
        llm: &dyn LlmAdapter,
        detail: &str,
    ) -> Result<TurnOutcome, crate::symbio_core::AppendError>
    where
        S: Store<Event = Event> + ?Sized,
    {
        exceed_steps(&mut self.run, store, llm, detail).await
    }
}

/// 一轮的步数上界（D-115 决策 3）："停机判据"而不是性能阀。
///
/// 超过它 = 适配器 / 分流在打转（每轮都回「继续」），而每轮都真花一次模型往返。
/// 取 64：工具循环的合法轮数在个位数，两个数量级的余量足够；再大就只是把
/// 一个卡死的轮拖得更久。
const MAX_STEPS_PER_DRIVE: usize = 64;

/// 步数到顶时的收束：落兜底格但 `reason = exhausted`（与截断同一闭集值）。
///
/// 半截产出照样带进载荷（它们是事实，不因停在这里就丢）。
async fn exceed_steps<S>(
    run: &mut prepare::Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    detail: &str,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let cost = run.started.elapsed().as_millis() as u64;
    let facts = facts::node_facts(&run.nodes);
    let produced = facts::produced_payload(
        &facts.text.clone().unwrap_or_default(),
        llm.model_id(),
        &run.turn_root_id,
        &facts,
        &run.all_tool_calls,
        &run.rounds,
    );
    settle::append_exhausted(
        store,
        &run.actor,
        run.turn,
        run.user_seq,
        detail,
        cost,
        produced,
        run.artifacts_landed,
    )
    .await
}

/// 收束格的**种类**（[D-065] 裁决、[D-067] 落地形态）：同一道写侧闸按种类分派。
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
/// 决策 D-065 见 [D-065](../../../../docs/003-design/011-Agent目录规范.md#d-065-对等侧收束commitmentasserted-缺的不是写方而是对等-offerassert②-与-③-同一批收束格按种类判 "D-065")
/// 决策 D-067 见 [D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "D-067")
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

/// 写侧授权闸（[plan/01 §7](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 写侧，
/// [04 §3.1 批⑥](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：该主体此刻能否写
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
/// ## 第二道：作用域上限（[]）
///
/// 主体自己那一行之外还有一道——**委派不放大权威**：`scope = child <parent>` 的
/// 主体（`agent_run` 派生的子会话）能落的格，其**父主体也必须能落**。
/// `root` = 独立主体，无上限。语义与取值点见
/// [ §3.2](../../../../docs/003-design/038-多主体与对等承诺.md "DE-038-多主体与对等承诺")
/// 与 `chat_loop::state::request_scope`。
///
/// 平凡值下（顶层会话 = `root`，以及主智能体派生的子会话）两道都是恒真——
/// 闸是 fail-closed 的**结构**，不是生产里会翻面的开关。
///
/// ## 为什么带出原因
///
/// 拒绝原因（`authz.denied` 的 `detail`）答的是「缺哪项能力」——判据与判据
/// 同源，只多带一句人话。布尔形态（`is_ok()` 壳）已删：生产面七处收束落格
/// 都要原因，没人再取「过没过」这个形状。
fn closure_gate(actor: &ActorSpec, kind: ClosureKind) -> Result<(), String> {
    // 八个收束调用点全在「本轮尚无收束格」的位置，因此发言只问一个 `Capability::Reply`
    // ——名住 core，规格方法与表谓词共用它（D-103：不按轮次位置分能力）。
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

/// 落一条**否定事实**（[`EVENT_AUTHZ_DENIED`]，）。
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
/// 义务环落地后它会换成 `Pattern` 的键（DE-015 §4.2 的 `denied(key)`）——今天没有
/// `Decl`，先用格名，那是同一件事在日志里的名字。
/// 落一条**否定事实**（[`EVENT_AUTHZ_DENIED`]，）。
///
/// 载荷 `{ key, kind, reason, detail }`：`reason` 取 [`ClosureReason::Denied`]——
/// 与收束格**同一个字段名、同一个闭集**，于是读侧能用**一个**问题（「这一轮为什么停」）
/// 问遍两种格，不必先猜这是收束格还是否定事实。`detail` 是人话，只给人读。
fn deny<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    kind: ClosureKind,
    key: &str,
    detail: &str,
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
        "reason": crate::symbio_core::ClosureReason::Denied.as_str(),
        "detail": detail,
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

/// 作用域上限（[]）：`scope = child <parent>` ⇒ 父主体也必须能落这一类格。
///
/// 谓词与 [`closure_gate`] 的**同一份**（`can_reply` / `can_produce_artifact`）
/// ——两处各写一套「这一类格要什么能力」就会让上限与许可分叉。
fn within_delegation_ceiling(actor: &ActorSpec, kind: ClosureKind) -> Result<(), String> {
    let crate::symbio_core::Scope::Child(parent) = &actor.scope else {
        return Ok(());
    };
    let matrix = crate::symbio_core::production_matrix();
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
