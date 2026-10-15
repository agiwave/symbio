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
//! `grep -c TurnRunner docs/plan/PL-001-核心架构.md` = 0。真正的冻结锚点
//! （[plan/03 §1](../../../../docs/plan/PL-003-演进与验证.md "PL-003-演进与验证")）是：F1 `Store` 签名 /
//! F2 `Projection` 构造形状 / **F3 `ActorSpec` 字段集 + 档位→令牌映射** /
//! F4 三条不变量 / F5 三个封闭集合 / F6 参数表键集。**`TurnRunner` 不在其中**，
//! 所以这次下沉**不改 F1–F6**，按 [plan/03 §2](../../../../docs/plan/PL-003-演进与验证.md "PL-003-演进与验证")
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
    PromptToolCall, Seq, Store, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_ASSISTANT_OPENING, EVENT_ASSISTANT_PROGRESS, EVENT_SESSION_CONSOLIDATED,
    EVENT_USER_MESSAGE,
};
use std::borrow::Cow;

// ── v2 会话运行时：turn 运行器（chat_loop 切换的第一块可复用件）────────
//
// 职责把 [plan/04 §2](../../../../docs/plan/PL-004-工程落地.md "PL-004-工程落地") 的单轮流程落成
// 一个函数调用：用户消息入格 → [`Reasoner`] 生成（实测耗时）→ final / fallback
// 落格（I3：失败也必须有输出）。不变量靠构造成立：每轮独立 turn 号（N3）、
// final 溯源指向本轮用户消息（N5）、实测 cost_ms 随事件入账（ADR-044）。
// 会话轮与彩排共用本运行器：会话轮由 chat_loop 经 [`super::v2_exec`] 驱动，
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
    /// 那一刻恰好落在闸门与请求之间）。此前这一档由 `chat_loop` 在进运行器**之前**
    /// 拦下（`TurnExit::Aborted`）——运行器于是从不落 `user.message` 格，WAL 全空，
    /// 中止这一轮**一条事实都没有**（实测 e2e t7：`v2-events.wal` 不存在，
    /// `恢复后重建的历史里看不到这轮说过什么`）。而 ADR-052 决策 1 要求
    /// 「一轮的产出无论完成、失败、被中断还是悬在等待用户，都入格」。
    ///
    /// 修法 = 把检查点**下沉**到运行器：落完用户格后、发第一个请求前检查一次，
    /// 已中止则直接走**中止收束格**（`chat.assistant.final` + `status: "aborted"` +
    /// `terminated`）——与 adapter 中途返回 `AdapterError::Aborted` 落的是同一格。
    ///
    /// `None` = 调用方不提供信号（单测 / 无中止语义的路径），行为与接线前逐字一致。
    pub abort: Option<crate::symbio_core::ExecAbortSignal>,
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
    /// `viewer` 都取它（[plan/11 批 1](../../../../docs/plan/PL-009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
    /// ①：`"agent:main"` 从字面量变成**入参**，`ActorSpec` 首次在生产构造）。
    ///
    /// 身份是数据不是常量：同一个会话引擎跑在哪个 agent 上，由**调用方**说了算
    /// （`plugins/session/v2_exec` 从会话元数据派生）。S08 §4 的平凡值是
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
    /// `merge_supplements`、`ChatMessage` 合并都在 `plugins/session`）。于是这个口
    /// 收的是**已经折好的消息**（`PromptMessage` 本来就存在），落格也在插件侧做
    /// （`v2_exec` 自己开着 `EventWalStore`）。
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
    /// 双写期它的形状随整份 `messages.json` 落盘；切源后收束格只记了正文
    /// （`final_reply`），重建便**折不出这条节点**——实测 t20 / t21 / t22 / t25 全部
    /// 「0 条对话面节点」。节点形状是事实的一部分（T-052 原则），故随收束格一并入格。
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
    /// 前端据它认出「这是一次折进」，重建据它复刻节点形状。双写期这些键随整份
    /// `messages.json` 落盘，切源后若不随事实入格，重建出的用户节点就**丢了这些键**，
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
    /// [`super::message_build::tool_call_args_text`] 同一取法）。
    pub args: String,
    /// provider wire id（`tool_call_id` 字段——节点身份的一部分）。
    pub wire_id: String,
    /// 结果子节点 id（`None` = 尚无结果：无分发通道 / 等待用户 / 恢复路）。
    pub result_id: Option<String>,
}

/// 取此刻的流式节点事实（无桥 ⇒ 空事实，不是错误）。
fn node_facts(nodes: &Option<NodeFactsFn>) -> StreamNodeFacts {
    nodes.as_ref().map(|f| f()).unwrap_or_default()
}

/// 产出部分的载荷（收束 / 进展 / 兜底格共用的形状）。
///
/// 一处定义的理由：三种格子记的是**同一批产出事实**（末轮正文 / 思考 / 本轮全部
/// 工具调用 / 工具轮的中途产出），差别只在状态字段——形状写三遍必然漂移，而重建
/// 只按一个形状读。半截正文 / 半截思考照原样进 `text` / `reasoning` / `rounds`：
/// **它们是事实**，不因「没说完」就不记。
fn produced_payload(
    text: &str,
    model: &str,
    turn_root_id: &str,
    facts: &StreamNodeFacts,
    tool_calls: &[ToolCallFact],
    rounds: &[StreamNodeFacts],
) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        // 空串 = 这一轮没有模型参与（定稿答话 / 反射档），不是「忘了记」。
        "model": model,
        // 节点结构入格：重建按这批 id 复刻落库形状（载荷补全）。
        "turn_id": turn_root_id,
        "text_id": facts.text_id,
        "reasoning_id": facts.reasoning_id,
        "reasoning": facts.reasoning,
        "tool_calls": tool_calls_payload(tool_calls),
        "rounds": rounds_payload(rounds),
    })
}

/// `tool_calls` 载荷条目（形状见 [`ToolCallFact`]）。
fn tool_calls_payload(calls: &[ToolCallFact]) -> Vec<serde_json::Value> {
    calls
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "name": c.name,
                "args": c.args,
                "wire_id": c.wire_id,
                "result_id": c.result_id,
            })
        })
        .collect()
}

/// `rounds` 载荷条目：工具轮的**中途产出**（每轮正文 / 思考 + 节点 id）。
fn rounds_payload(rounds: &[StreamNodeFacts]) -> Vec<serde_json::Value> {
    rounds
        .iter()
        .map(|r| {
            serde_json::json!({
                "text_id": r.text_id,
                "text": r.text,
                "reasoning_id": r.reasoning_id,
                "reasoning": r.reasoning,
            })
        })
        .collect()
}

/// 一次工具调用 → 事实条目（结果配不上的记 `result_id: None`）。
fn tool_fact_of(
    tc: &crate::symbio_core::TurnToolCallInfo,
    outcome: Option<&crate::symbio_core::DispatchOutcome>,
) -> ToolCallFact {
    ToolCallFact {
        id: outcome
            .map(|o| o.call_id.clone())
            .or_else(|| tc.id.clone())
            .unwrap_or_default(),
        name: tc.name.clone().unwrap_or_default(),
        args: super::message_build::tool_call_args_text(tc),
        wire_id: outcome
            .map(|o| o.wire_id.clone())
            .or_else(|| tc.wire_id.clone())
            .unwrap_or_default(),
        result_id: outcome
            .map(|o| o.result_id.clone())
            .filter(|s| !s.is_empty()),
    }
}

/// 从本轮结果里配对出 `tc` 的那条（按 ToolCall 节点 id / wire id；配不上 ⇒ `None`）。
fn outcome_of<'a>(
    outcomes: &'a [crate::symbio_core::DispatchOutcome],
    tc: &crate::symbio_core::TurnToolCallInfo,
) -> Option<&'a crate::symbio_core::DispatchOutcome> {
    outcomes.iter().find(|o| {
        tc.id.as_deref().map(|id| id == o.call_id).unwrap_or(false)
            || (!o.wire_id.is_empty() && tc.wire_id.as_deref() == Some(o.wire_id.as_str()))
    })
}

/// 给产出载荷补**状态**字段（完成 / 终止 / 等待是事实的状态，不进产出形状本身）。
fn with_status(mut produced: serde_json::Value, status: &str) -> serde_json::Value {
    produced["status"] = serde_json::Value::String(status.to_string());
    produced
}

/// 把定稿答话的**对话面节点身份**并入收束格载荷（见 [`TurnInput::dialog`]）。
///
/// 缺席（`None` = 这一轮不是答话轮）⇒ 载荷原样返回：普通收束格的重建路径不受影响，
/// 「有对话面节点」因此是一个**由载荷显式声明**的事实，不靠重建去猜。
fn with_dialog(mut produced: serde_json::Value, dialog: Option<&DialogFacts>) -> serde_json::Value {
    if let Some(d) = dialog {
        produced["dialog"] = serde_json::json!({
            "id": d.id,
            "reason": d.reason,
            "exclude_from_context": d.exclude_from_context,
        });
    }
    produced
}

/// 终止信息（`terminated: { reason, at }`，仅终止时有）：终止的事实被截到哪一刻、
/// 被什么截断——半截正文停在哪里由此可观测。
fn with_terminated(mut payload: serde_json::Value) -> serde_json::Value {
    payload["terminated"] = serde_json::json!({
        "reason": "aborted",
        "at": crate::symbio_core::clock_now_ms(),
    });
    payload
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
    /// 照样是事实；恢复是 [plan/11 批 2](../../../../docs/plan/PL-009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") 的
    /// 能力，恢复后以收束格了结（进展格不占「一轮至多一条收束格」）。
    pub awaits_user: bool,
    /// 本轮收束**被写侧闸拒绝**（主体不持 `reply.first`，见 [`closure_granted`]）：
    /// 收束格未入格、且本轮**没有落过产物格**，该轮留在未收束态（`check_all` 的 C4
    /// 报得出）。与 `aborted` / `awaits_user` 同属「网格少一格」，但**原因在授权**，
    /// 不是放弃也不是等待——调用方据此**跳过本轮的派生事实**（承诺 / 任务 / 熔断 /
    /// 记忆全不写）。
    ///
    /// **落过产物格 ⇒ 不算拒**（[ADR-067](../../../../docs/adr/ADR-067.md "ADR-067")）：
    /// 产物收束是另一条收束路（`ClosureKind::Artifact`），它落过格就说明这一轮有话
    /// 说过——典型是只持 `produce.artifact` 的对等主体：发言闸拒它，但它产出的
    /// 工具结果是事实，派生事实照写。
    ///
    /// 平凡值 `false`：生产里主体恒为 `agent:main` 或经 `matrix_for` 派生的
    /// `agent:<id>`，都持 `reply.*` ⇒ 闸是 fail-closed 的**结构**，不是会翻面的开关。
    pub closure_denied: bool,
}

/// 按主体过滤一批事件（**可见域入口**，[plan/11 批 1](../../../../docs/plan/PL-009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
/// ③）：`None` = 不过滤（与接线前逐字一致），`Some(v)` = 只交 `visible_to` 通过的事件。
///
/// 全部可见时**不复制**（借用原切片）——平凡值下（单主体会话）这条路径与
/// 接线前逐字一致，零拷贝。
fn filter_visible<'a>(events: &'a [Event], viewer: Option<&str>) -> Cow<'a, [Event]> {
    let Some(viewer) = viewer else {
        return Cow::Borrowed(events);
    };
    if events.iter().all(|e| visible_to(&e.actor, viewer)) {
        Cow::Borrowed(events)
    } else {
        Cow::Owned(
            events
                .iter()
                .filter(|e| visible_to(&e.actor, viewer))
                .cloned()
                .collect(),
        )
    }
}

/// 对话窗口：保留 turn 号落在「当前轮往前数 `keep` 个」之内的事件
/// （含当前轮）。事实是全量的，**视图**才是窗口——纯切片，不改数据。
///
/// `keep == 0` = **不截断**（全量）——与 `SessionConfig::context_messages` 的 `0`
/// 同口径。会话轮必填窗口值正是为了防「WAL 只增不减 ⇒ prompt 无界增长」，而
/// 「配 0 换不截断」是这个防护的**显式关闭开关**，不是笔误。
///
/// 第二层是**可见域**（`viewer`，见 [`filter_visible`]）：窗口先按轮切，
/// 再按主体滤——两层都只改视图，不改数据。
///
/// ## 压缩快照**不在窗口内也要带上**（S2.7-b2）
///
/// `session.consolidated` 是当前历史的**基线**（它的载荷就是压缩后的整份列表），
/// 窗口裁的是「更早的轮次」，不是基线本身。而段归档之后 live 段只剩
/// 「快照事件 + 压缩后的新事件」，且快照事件的 `turn` 不在窗口内 ⇒ 纯按轮切会把
/// 它切掉，于是模型**既看不见摘要、也看不见刚聊过什么**（被保留的近轮也在快照
/// 载荷里，不在 live 段里）。
///
/// 故窗口 = `[最近一次快照] ++ 窗口内的事件`。只带**最近**一条：更早的快照连它
/// 自己的列表都已被后来的快照取代（与投影 [`crate::symbio_core::transcript`] 同一口径）。
fn window_by_turn<'a>(
    events: &'a [Event],
    current_turn: u64,
    keep: u64,
    viewer: Option<&str>,
) -> Cow<'a, [Event]> {
    if keep == 0 {
        // `keep == 0` = **不截断**（全量）——与 `SessionConfig::context_messages`
        // 的 `0` 同一条口径（那里 `0` 一直是「上下文窗口不按轮次截断」）。两处
        // 对同一个数必须同义：否则配 `0` 的实例会**静默**丢掉全部历史
        // ——事实照样入格、`session/stats` 照样有数，只有模型看不见。
        return filter_visible(events, viewer);
    }
    let oldest = current_turn.saturating_sub(keep - 1);
    let cut = events
        .iter()
        .position(|e| e.entity == Entity::Turn && e.turn >= oldest)
        .unwrap_or(events.len());
    match latest_snapshot_index(events) {
        // 快照在窗口外 ⇒ 单独把它拎到最前（见函数文档）。
        Some(ci) if ci < cut => {
            let mut w: Vec<Event> = Vec::with_capacity(1 + events.len() - cut);
            w.push(events[ci].clone());
            w.extend_from_slice(&events[cut..]);
            if let Some(v) = viewer {
                w.retain(|e| visible_to(&e.actor, v));
            }
            Cow::Owned(w)
        }
        _ => filter_visible(&events[cut..], viewer),
    }
}

/// 切片里**最近一次**压缩快照（`session.consolidated`）的下标。
fn latest_snapshot_index(events: &[Event]) -> Option<usize> {
    events.iter().rposition(|e| {
        e.entity == Entity::Message
            && e.verb == Verb::Progressed
            && e.kind == EVENT_SESSION_CONSOLIDATED
    })
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
    ///   `produced_by` 指向**本轮用户格**（S02 §3 的 `caused_by` 断言）；
    /// - 收束：`chat.assistant.final`（或失败的兜底格）——**只落一次**。
    ///
    /// ## prompt 的两段
    ///
    /// 基线由 [`crate::symbio_core::actors::render_messages`] 从转写投影出（历史来自事实源，含**往轮**的
    /// 工具结果——[plan/10 批 3](../../../../docs/plan/PL-008-工具轮v2化实施方案.md "PL-008-工具轮v2化实施方案") 起）；
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
    /// 不收束轮次（[plan/11 批 2](../../../../docs/plan/PL-009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
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
        S: Store<Event = Event>,
    {
        let TurnInput {
            turn,
            text,
            tier,
            window_turns,
            inject,
            resume,
            actor,
            prefix,
            final_reply,
            dialog,
            opening_text,
            opening_dialog,
            turn_root_id,
            user_node_id,
            user_meta,
            nodes,
            abort,
        } = input;
        // 本轮的 viewer：历史窗口只交**本主体看得见**的事件（plan/11 批1 ③）。
        // 身份是入参不是字面量——`actor.principal` 由调用方（生产：会话属于哪个
        // agent）给出；S08 §4 平凡值 `agent:main` 下没有任何事件被滤掉。
        let viewer = Some(actor.principal.as_str());
        // 本轮**新落**的产物格数（[ADR-067] 的「产物收束」判据）：`> 0` ⇒ 本轮已
        // 收束，发言闸再拒也不算未收束（`TurnOutcome::closure_denied` 与 C4 都认它）。
        // 等待轮落过的不算——这里只数**本次执行**新落的；开在函数顶是因为 1b 定稿
        // 答话分支在续写落产物**之前**就要用它收口。
        let mut artifacts_landed = 0u64;
        // 1. 用户消息入格（turn × opened），档位随载荷入账。
        //
        // 续写轮（`resume`）**不新开用户格**：用户没再说话，重开会让同一句话在网格里
        // 出现两次、把那一轮数成两轮；且 C4 按 turn 号配对——续写轮要的正是让**原轮**
        // 收到收束事件（见 [`TurnResume`]）。此时复用该轮既有的用户格 seq 作溯源锚点。
        let user_seq = match &resume {
            None => {
                // 载荷：正文 + 档位 + 节点身份（id / meta）。`meta` **按需带上**——
                // 空对象不入格，与落库那些「没有 meta 就不序列化该键」的节点同形
                // （前端按 id 合并、重建按 `meta` 复刻形状，两者都读这份载荷）。
                let mut payload = serde_json::json!({
                    "text": text,
                    "tier": tier.name(),
                    // 用户消息节点 id 入格：重建按它复刻节点身份（前端按 id 合并）。
                    "id": user_node_id,
                });
                if let Some(meta) = user_meta.as_ref().filter(|m| !m.is_null()) {
                    payload["meta"] = meta.clone();
                }
                store
                    .append(
                        Event::pending(
                            format!("u-{turn}"),
                            EVENT_USER_MESSAGE,
                            Entity::Turn,
                            Verb::Opened,
                            turn,
                            "user",
                        )
                        .with_payload(payload),
                    )
                    .map(|seq| seq.value())?
            }
            Some(r) => r.user_seq,
        };
        // 1a. 对话面开场白（T-056）：`Escalate` 首响——本轮随后**照进工具循环**，
        // 所以它落在 `turn × asserted`（不是收束格 N3「每轮至多一条」）。
        //
        // 放在用户格之后：断言类事件必须带溯源锚点（I2），而那只有落完用户格才有。
        // 空正文不落格（与汇报 / 收束同一条纪律）：把空串记成「说了句开场白」会让
        // 前端多出一条空节点。
        if let (Some(opening), Some(dialog)) = (opening_text.as_deref(), opening_dialog.as_ref()) {
            if !opening.trim().is_empty() {
                store.append(
                    Event::pending(
                        format!("op-{turn}"),
                        EVENT_ASSISTANT_OPENING,
                        Entity::Turn,
                        Verb::Asserted,
                        turn,
                        &actor.principal,
                    )
                    .with_produced_by(user_seq)
                    .with_payload(serde_json::json!({
                        "text": opening,
                        "id": dialog.id,
                        "reason": dialog.reason,
                        "exclude_from_context": dialog.exclude_from_context,
                    })),
                )?;
            }
        }
        // 1b. 定稿答话（缺口 5）：用户格照常开，答话照常收束，**模型不参与**。
        //
        // 为什么放在这里而不是循环里：这一轮一次请求都不发，循环的第一件事
        // （渲染 prompt、调模型）对它全都是白工。放在用户格之后是因为收束格的
        // 溯源必须指向本轮用户格（I2）——而那只有落完用户格才有。
        //
        // 空答话与模型返回空文本同一条纪律：不落收束，落到兜底话术上。把空串
        // 记成「答了空话」会让「到点必答」这条线无声失效（I3）。
        if let Some(reply) = final_reply.as_deref() {
            let facts = node_facts(&nodes);
            // 产出部分的载荷一处构造（收束 / 进展 / 兜底共用同一形状），状态字段由
            // 各落格分支自己补——状态是事实的状态，不进产出形状本身。
            let produced = produced_payload(reply, "", &turn_root_id, &facts, &[], &[]);
            if reply.trim().is_empty() {
                // 耗时记 0：这一轮一次模型请求都没发（答话是调用方给好的
                // `final_reply`），`started` 那条计时口径本来就是「模型往返」。
                return Self::append_fallback(
                    store,
                    &actor,
                    turn,
                    user_seq,
                    "compose returned empty",
                    0,
                    produced,
                    artifacts_landed,
                )
                .await;
            }
            if !closure_granted(&actor, ClosureKind::Speech) {
                crate::plugin_warn!(
                    "actors",
                    "[v2] 定稿答话被授权拒绝（{} 缺 reply.first），本轮不入格（turn={turn}）",
                    actor.principal
                );
                return Ok(TurnOutcome {
                    turn,
                    text: reply.to_string(),
                    cost_ms: 0,
                    usage: None,
                    fell_back: false,
                    aborted: false,
                    awaits_user: false,
                    // 落过产物格 ⇒ 本轮已收束（产物收束，ADR-067）：不算拒。
                    closure_denied: artifacts_landed == 0,
                });
            }
            store.append(
                Event::pending(
                    format!("f-{turn}"),
                    EVENT_ASSISTANT_FINAL,
                    Entity::Turn,
                    Verb::Closed,
                    turn,
                    &actor.principal,
                )
                .with_produced_by(user_seq)
                .with_cost_ms(0)
                .with_payload(with_dialog(
                    with_status(produced, "completed"),
                    dialog.as_ref(),
                )),
            )?;
            return Ok(TurnOutcome {
                turn,
                text: reply.to_string(),
                cost_ms: 0,
                usage: None,
                fell_back: false,
                aborted: false,
                awaits_user: false,
                closure_denied: false,
            });
        }
        let full_snapshot = store.range(Seq::new(0));
        // 对话窗口：只把最近 N 个 turn 的事件交给 prompt（含当前轮）——
        // 窗口是**视图**问题，事实照常全量入格（append-only 不受影响）。
        let snapshot: Cow<'_, [Event]> = match window_turns {
            None => filter_visible(&full_snapshot, viewer),
            Some(keep) => window_by_turn(&full_snapshot, turn, keep, viewer),
        };
        // 基线：**结构化**消息数组（ADR-048a）。
        //
        // prefix（请求视图三段：记忆召回 / 就绪任务 / 委派者真源）作为**最前的
        // 一条独立消息**插进去，而不是拼进第一段正文——拼进去的话它会与用户本轮
        // 的原话混成一句，模型分不清哪部分是系统给的背景、哪部分是用户说的。
        //
        // ## 为什么 role 是 `system` 而不是 `user`
        //
        // 标成 `user` 就是**在说「这是用户说的话」**，而它是系统注入的读视图。
        // 这个错有实测代价：e2e 的 mock 按「最后一条 user 消息 = 本轮用户发言」
        // 选场景，于是 prefix 顶掉用户原话 ⇒ 场景匹配全落空 ⇒ t36 报「恰好一次
        // 触发，实得 2 次」（两次都拿到兜底场景）。**心跳轮本来没有用户发言**，
        // 所以那里 prefix 成了唯一一条 user —— 错得最彻底。
        //
        // 标成 `system` 同时解决两件事：模型知道这是背景（不当作用户指令），
        // 而「本轮用户说了什么」在消息层上**不再有歧义**。
        let mut base_messages = crate::symbio_core::render_messages(&snapshot);
        if let Some(p) = prefix.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            base_messages.insert(
                0,
                crate::symbio_core::PromptMessage {
                    role: "system".into(),
                    text: p.to_string(),
                    tool_call_id: None,
                    tool: None,
                    tool_calls: None,
                },
            );
        }

        // **双向完整性复核**（ADR-048 防线）：送进模型的每条消息都必须能在网格里
        // 找到出处，且 `role` 完好。缺口只出声不抛错——prompt 已经发出去了，此时抛错
        // 只会把「模型看得不全」变成「这一轮直接失败」；而**看得不全恰恰是静默的**
        // （不报错、不告警、既有断言全绿），所以这里必须出声。
        //
        // `declared` 是**按设计**不进网格的那一条（prefix）。它是**声明过的例外**
        // 而不是「判据放宽」：三段请求视图是**读视图**（本次请求临时投影出来的），
        // 不是事件，所以它**理应**报「无出处」——但把已知的那条一并放过，判据才
        // 有用。**一个永远红的守卫等于没有守卫。**
        {
            let declared = prefix.as_deref().map(str::trim).filter(|p| !p.is_empty());
            let rep =
                crate::symbio_core::verify_prompt_fidelity(&snapshot, &base_messages, declared);
            if !rep.is_complete() {
                crate::plugin_warn!(
                    "actors",
                    "[prompt-fidelity] 送出的消息与事实网格不完整：{}",
                    rep.summary()
                );
            }
        }

        let started = std::time::Instant::now();
        // 本轮内已发生的工具交换（调用 + 结果），供下一次请求追加。
        //
        // **结构化**（ADR-048a）：assistant（带 `tool_calls`）+ tool（带 `tool_call_id`）。
        // 拍成散文会让模型把工具调用当成「自己说的话」、把结果当成无角色文本，且丢掉
        // `tool_call_id`（协议要求它配对）——所以这里是真消息，不是一段文本。
        let mut exchange: Vec<crate::symbio_core::PromptMessage> = Vec::new();
        // 本回合内已走完的**工具轮数**——只用于给合成 id 编号（见 [`prompt_call_id`]）：
        // 交换跨轮累加，id 的唯一性域随之从一次请求扩到整个回合。
        let mut tool_round: usize = 0;
        // 实测耗时：跨轮累加（一次用户轮可能有多次 LLM 请求，ADR-044 的实测口径）。
        let mut cost_ms = 0u64;
        // 产物格的事件 id 需要在本轮内唯一（同一工具可被调用多次）。续写轮从**该轮
        // 已有产物数**起编号——等待轮可能已落过产物格，从 0 起会撞幂等键（`Duplicate`）。
        let mut artifact_no = full_snapshot
            .iter()
            .filter(|e| e.turn == turn && e.kind == crate::symbio_core::EVENT_ARTIFACT_ADDED)
            .count() as u64;
        // 本轮**全部**工具调用的结构（跨工具轮累积）——收束 / 进展事件载荷记它，
        // 重建才有「这一轮请求过什么」的完整事实（载荷补全）。
        let mut all_tool_calls: Vec<ToolCallFact> = Vec::new();
        // 工具轮的**中途产出**（每轮正文 / 思考的节点事实，跨轮累积）——末轮之外的
        // 每一轮都进 `rounds`，收束 / 进展载荷记它（半截正文 / 半截思考照入格）。
        let mut rounds: Vec<StreamNodeFacts> = Vec::new();

        // 续写轮：恢复产生的工具交换是**本轮已发生的事实**——先落格（`artifact.added`，
        // 溯源指向本轮用户格，I2）再拼进 prompt。落格之后它固然进了投影，但本轮的
        // `base_prompt` 已在上方渲染完毕（快照早于这次 append），故仍就地追加——模型
        // 必须看到「我请求了什么、拿到了什么」，否则会重复调用同一个工具。
        if let Some(r) = &resume {
            // 产物闸（`ClosureKind::Artifact`）：不持 `produce.artifact` ⇒ 这一格不落。
            // 结果**照进 prompt**（模型看得见）+ 照记 `all_tool_calls`（请求发生过）——
            // 闸管的是「入格」，不是「发生过」。
            if closure_granted(&actor, ClosureKind::Artifact) {
                store.append(
                    Event::pending(
                        format!("a-{turn}-{artifact_no}"),
                        crate::symbio_core::EVENT_ARTIFACT_ADDED,
                        Entity::Artifact,
                        Verb::Asserted,
                        turn,
                        &actor.principal,
                    )
                    .with_produced_by(user_seq)
                    .with_payload(serde_json::json!({
                        "tool": r.call.name.clone().unwrap_or_default(),
                        "text": r.text,
                        // 节点结构入格：ToolCall / 结果节点的身份与参数（重建复刻同一批）。
                        //
                        // 恢复路的两张卡（调用与结果）在实时面已经由 `resume.rs` 分配了 id
                        // （`Born{Carrier::Runner}`：节点在、但出生事实由**本格**承载）——
                        // 载荷复用**同一批 id**，重建才复刻出同一条节点。缺 `tool_call_id`
                        // 会让重建按事件 id 合成一个新调用 ⇒ 原调用成孤儿（e2e t27）；
                        // 缺 `result_id` 会让结果节点换个身份（前端按 id 合并 ⇒ 多一条）。
                        "tool_call_id": r.call_id.clone().or_else(|| r.call.id.clone()),
                        "wire_id": r.call.wire_id,
                        "args": super::message_build::tool_call_args_text(&r.call),
                        "result_id": r.result_id,
                        "success": true,
                        // 调用节点落回**原来那一轮**的 Turn 根：续写不新开轮，也不该长出
                        // 第二个 Turn 根。空缺（老调用点）时回落到本轮的根，与接线前同形。
                        "turn_id": r.turn_id.clone().unwrap_or_else(|| turn_root_id.to_string()),
                    })),
                )?;
                artifact_no += 1;
                artifacts_landed += 1;
                all_tool_calls.push(ToolCallFact {
                    id: r
                        .call_id
                        .clone()
                        .or_else(|| r.call.id.clone())
                        .unwrap_or_default(),
                    name: r.call.name.clone().unwrap_or_default(),
                    args: super::message_build::tool_call_args_text(&r.call),
                    wire_id: r.call.wire_id.clone().unwrap_or_default(),
                    // 结果子节点身份 = 恢复方分配的那一个（随载荷入格，见上）。
                    result_id: r.result_id.clone(),
                });
            } else {
                crate::plugin_warn!(
                    "actors",
                    "[v2] 产物被授权拒绝（{} 缺 produce.artifact），本条结果不入格（turn={turn}）",
                    actor.principal
                );
            }
            exchange.extend(exchange_messages(&r.call, &r.text));
        }

        // 1c. **闸门与首请求之间的中止**（ADR-052 决策 1）。
        //
        // 用户按下停止的那一刻可能恰好落在「用户格已落、第一个 LLM 请求还没发」这段
        // 空隙里。此前这一段由 `chat_loop` 在**进运行器之前**拦下（`TurnExit::Aborted`），
        // 于是运行器从不落 `user.message`，中止这一轮**一条事实都没有**——WAL 全空，
        // 溯源锚解不出，投影里看不到用户说过什么（实测 e2e t7）。
        //
        // 修法 = 检查点**下沉**到这里：用户格已落、请求未发，已中止则直接走**中止收束格**
        // （`chat.assistant.final` + `status: "aborted"` + `terminated`），与 adapter 中途
        // 返回 `AdapterError::Aborted` 落的是**同一格**——终止是事实的**状态**，不是新种类。
        // 半截产出此时为空（还没发请求），载荷照样带上（`produced_payload` 对空正文是合法的）。
        //
        // `abort` 为 `None`（单测 / 无中止语义的路径）时这一整段是空操作，行为与接线前逐字一致。
        if let Some(sig) = abort.as_ref() {
            if sig.is_aborted() {
                return Self::append_aborted(store, &actor, turn, user_seq, llm, &turn_root_id)
                    .await;
            }
        }

        loop {
            // 基线 + 本轮交换。**拼接消息数组**：拼字符串会把角色拍平、丢掉 `tool_call_id`。
            let mut messages = base_messages.clone();
            messages.extend(exchange.iter().cloned());

            // 2. 生成（实测耗时在 adapter 边界取得；失败路径的耗时从调用起点算）。
            match llm.generate_turn(tok, &messages, tools, sink.clone()).await {
                Ok(rt) => {
                    cost_ms += rt.cost_ms;
                    // 每轮的流式节点事实在**分发之前**取定：工具分发的定稿会重置桥，
                    // 之后再取只拿得到下一轮的节点——本轮的中途正文 / 思考就静默丢了。
                    let round_facts = node_facts(&nodes);

                    // 2a. 本轮不再请求工具 ⇒ 收束（落 final）。
                    if rt.tool_calls.is_empty() {
                        // 本轮的回答 = 正文，正文空则取**思考**（v1 的
                        // `TurnOutput::effective_text` 同一语义）。
                        //
                        // ⚠️ 判据不能只看 `rt.text`：reasoning 模型常常「想清楚了但不
                        // 复述」——末轮只有 `reasoning_content`、`content` 为空，那是
                        // **一句回答**（思考即正文）而不是「没答」。此前这里读不到思考
                        // （思考只经流式桥走另一条路），于是整轮被判失败、真答案连同全部
                        // 思考落进兜底格，界面上表现为「一串连续思考，没有正文」。
                        // 桥上那一份是第三选择：端口没带思考的适配器（默认实现只转正文）
                        // 只能靠它。
                        let answer = if !rt.text.trim().is_empty() {
                            rt.text.clone()
                        } else if !rt.reasoning.trim().is_empty() {
                            rt.reasoning.clone()
                        } else {
                            round_facts.reasoning.clone().unwrap_or_default()
                        };
                        if answer.trim().is_empty() {
                            // 正文与思考**都**空 + 无工具调用 = 模型什么也没答。这是
                            // **失败**：落成 final 会把「没答」记成「答了空话」（I3 的
                            // 到点必答要求有一句话，兜底话术承担它）。
                            let cost = started.elapsed().as_millis() as u64;
                            let produced = produced_payload(
                                &answer,
                                llm.model_id(),
                                &turn_root_id,
                                &round_facts,
                                &all_tool_calls,
                                &rounds,
                            );
                            return Self::append_fallback(
                                store,
                                &actor,
                                turn,
                                user_seq,
                                "model returned empty text",
                                cost,
                                produced,
                                artifacts_landed,
                            )
                            .await;
                        }
                        if !closure_granted(&actor, ClosureKind::Speech) {
                            // 写侧闸拒：**不落收束格**——该轮留在未收束态，`check_all`
                            // 会把它报出来（C4），与 [`closure_granted`] 同一条纪律、
                            // 同一个谓词。用户的答案照旧可见（`text` 仍回给调用方）。
                            //
                            // **例外**：本轮落过产物格 ⇒ 已由产物收束（ADR-067），
                            // `closure_denied` 为假，派生事实照写。
                            crate::plugin_warn!(
                                "actors",
                                "[v2] 收束被授权拒绝（{} 缺 reply.first），本轮不入格（turn={turn}）",
                                actor.principal
                            );
                            return Ok(TurnOutcome {
                                turn,
                                text: answer,
                                cost_ms,
                                usage: rt.usage,
                                fell_back: false,
                                aborted: false,
                                awaits_user: false,
                                closure_denied: artifacts_landed == 0,
                            });
                        }
                        let produced = produced_payload(
                            &answer,
                            llm.model_id(),
                            &turn_root_id,
                            &round_facts,
                            &all_tool_calls,
                            &rounds,
                        );
                        store.append(
                            Event::pending(
                                format!("f-{turn}"),
                                EVENT_ASSISTANT_FINAL,
                                Entity::Turn,
                                Verb::Closed,
                                turn,
                                &actor.principal,
                            )
                            .with_produced_by(user_seq)
                            .with_cost_ms(cost_ms)
                            .with_payload(with_status(produced, "completed")),
                        )?;
                        return Ok(TurnOutcome {
                            turn,
                            text: answer,
                            cost_ms,
                            usage: rt.usage,
                            fell_back: false,
                            aborted: false,
                            awaits_user: false,
                            closure_denied: false,
                        });
                    }

                    // 2b. 模型请求了工具 ⇒ 分发。没有分发方却收到工具调用是**配置
                    //     缺口**（无工具轮不该出现工具调用），按失败诚实回报。
                    let Some(dispatch) = dispatch else {
                        let cost = started.elapsed().as_millis() as u64;
                        // 调用本身是事实：没有分发通道也要记「请求过什么」
                        // （`result_id` 记 `None`——还没有结果，不是空结果）。
                        all_tool_calls
                            .extend(rt.tool_calls.iter().map(|tc| tool_fact_of(tc, None)));
                        let produced = produced_payload(
                            &rt.text,
                            llm.model_id(),
                            &turn_root_id,
                            &round_facts,
                            &all_tool_calls,
                            &rounds,
                        );
                        return Self::append_fallback(
                            store,
                            &actor,
                            turn,
                            user_seq,
                            "模型请求了工具，但本轮没有工具分发通道",
                            cost,
                            produced,
                            artifacts_landed,
                        )
                        .await;
                    };
                    let outcomes = dispatch.dispatch(&rt).await;

                    // 2c. 产物落格：每条工具结果一格，溯源指向本轮用户格。
                    //     节点结构入格（与恢复路同形）：结果正文归这一格（一个事实一个
                    //     owner），收束 / 进展载荷的 `tool_calls` 只记 `result_id` 引用。
                    // 本轮的中途产出（思考 / 正文）**随这一轮第一格产物入格**。
                    //
                    // ⚠️ 判据是**顺序**，不是「有没有记」：这些节点本来就记在收束格的
                    // `rounds[]` 里，一个字段都没少。但重建的顺序锚是**格的发射顺序**
                    // （`seq` 由 `nodes_of` 统一发号），收束格排在全轮末尾 ⇒ 重开一次
                    // 会话，中途思考就整体挪到全部工具卡片**之后**连成一串（实测会话
                    // fixverify01：实时面是「思考→卡片→思考→卡片」，历史面是 13 个思考
                    // 块排在 seq 29…43）。用户看到的「大模型连续的思考过程」是这条挪动，
                    // 不是模型的行为。
                    // 挂在产物格上 = 挂在这一轮发生的那个时刻；同一 id 只认首次出现的
                    // 一份，收束格 `rounds[]` 里的那些重复因此被去重（不删它：老会话的
                    // 收束格没有配套的产物格，删了就丢）。
                    let round_nodes = rounds_payload(std::slice::from_ref(&round_facts))
                        .pop()
                        .unwrap_or_else(|| serde_json::json!({}));
                    for (oi, outcome) in outcomes.iter().enumerate() {
                        // 产物闸（`ClosureKind::Artifact`，ADR-067）：不持
                        // `produce.artifact` ⇒ 这条结果不入格（warn 一次，不刷屏）。
                        // 结果**照进 prompt / 照记 `all_tool_calls`**——闸管的是
                        // 「入格」，不是「发生过」。
                        if !closure_granted(&actor, ClosureKind::Artifact) {
                            if oi == 0 {
                                crate::plugin_warn!(
                                    "actors",
                                    "[v2] 产物被授权拒绝（{} 缺 produce.artifact），本轮工具结果不入格（turn={turn}）",
                                    actor.principal
                                );
                            }
                            continue;
                        }
                        store.append(
                            Event::pending(
                                format!("a-{turn}-{artifact_no}"),
                                crate::symbio_core::EVENT_ARTIFACT_ADDED,
                                Entity::Artifact,
                                Verb::Asserted,
                                turn,
                                &actor.principal,
                            )
                            .with_produced_by(user_seq)
                            .with_payload(serde_json::json!({
                                "tool": outcome.name,
                                "text": outcome.text,
                                "tool_call_id": outcome.call_id,
                                "wire_id": outcome.wire_id,
                                "args": outcome.args,
                                "result_id": outcome.result_id,
                                "success": outcome.ok,
                                // 等待用户：结果节点是一张**待用户动作的卡**（`user_prompt` +
                                // `waiting_user_action`），不是普通结果文本。重建据此复刻
                                // 同一个节点形状与卡片载荷（T-056：读路径走 WAL 重建）。
                                "needs_user_action": outcome.needs_user_action,
                                "prompt": outcome.prompt,
                                "failure_kind": outcome.failure_kind,
                                "turn_id": turn_root_id,
                                // 本轮中途产出：只随**第一格**带一次（同一轮多张卡片时
                                // 思考只有一段，挂两处即两个 owner）。
                                "round": (oi == 0).then(|| round_nodes.clone()),
                            })),
                        )?;
                        artifact_no += 1;
                        artifacts_landed += 1;
                    }
                    // 本轮工具调用的结构进累积（配得上结果的记结果节点 id）。
                    all_tool_calls.extend(
                        rt.tool_calls
                            .iter()
                            .map(|tc| tool_fact_of(tc, outcome_of(&outcomes, tc))),
                    );

                    // 2d. 收束于等待用户 ⇒ 落**进展格**（`status: "pending"`：模型已
                    //     说过的话照样是事实，等待是状态），不收束轮次——恢复续写同一轮、
                    //     以收束格了结（见函数文档）。
                    if outcomes.iter().any(|o| o.needs_user_action) {
                        let produced = produced_payload(
                            &rt.text,
                            llm.model_id(),
                            &turn_root_id,
                            &round_facts,
                            &all_tool_calls,
                            &rounds,
                        );
                        // 进展格的事件 id 按**本轮已有进展格数**编号（同产物格的幂等
                        // 模式）：等待轮可能已落过进展格（恢复后又等待），从 0 起会撞
                        // 幂等键（`Duplicate`）。
                        let progress_no = full_snapshot
                            .iter()
                            .filter(|e| e.turn == turn && e.kind == EVENT_ASSISTANT_PROGRESS)
                            .count() as u64;
                        store.append(
                            Event::pending(
                                format!("p-{turn}-{progress_no}"),
                                EVENT_ASSISTANT_PROGRESS,
                                Entity::Turn,
                                Verb::Progressed,
                                turn,
                                &actor.principal,
                            )
                            .with_produced_by(user_seq)
                            .with_cost_ms(cost_ms)
                            .with_payload(with_status(produced, "pending")),
                        )?;
                        return Ok(TurnOutcome {
                            turn,
                            text: rt.text,
                            cost_ms,
                            usage: rt.usage,
                            fell_back: false,
                            aborted: false,
                            awaits_user: true,
                            closure_denied: false,
                        });
                    }

                    // 2e. 把这一轮交换**累加**进 prompt，继续下一轮。
                    //     本轮（工具轮）的中途产出进 `rounds`——它不是末轮，收束载荷的
                    //     顶层字段留给末轮 / 中止 / 兜底那一轮，避免同一段正文记两遍。
                    //
                    //     ⚠️ 这里是累加而不是**替换**：本回合此前每一次调用与结果都必须
                    //     留在下一次请求里（与本函数恢复路同一条不变量，见 932 行那段
                    //     「模型必须看到『我请求了什么、拿到了什么』，否则会重复调用
                    //     同一个工具」）。清掉交换等于让模型每轮重新开局——实测表现为
                    //     同一份文件读四遍、待办在 5 项与 6 项之间反复重置，最后模型
                    //     吐一轮空正文收进兜底格。
                    //     唯一性代价由 id 承担（见 [`prompt_call_id`]），不由失忆承担。
                    rounds.push(round_facts);
                    exchange.extend(tool_exchange_messages(&rt, &outcomes, tool_round));
                    tool_round += 1;

                    // 2f. 轮内注入：调用方可能在这时候有新东西要加进下一次请求
                    // （生产里是「用户中途补充」）。core **只把消息接上**——它不
                    // 知道那是什么、也不落格（落格在调用方，它自己开着 store）。
                    //
                    // 挂在 2e 之后而不是之前：工具交换先接上，注入的内容排在它
                    // 之后 —— **时间序**：工具结果先发生，用户随后才插话。
                    if let Some(inject) = &inject {
                        // 锚点 = 本轮用户格 seq（`user_seq` 在开轮那步就拿到了）。
                        exchange.extend(inject(Some(user_seq)).await);
                    }
                }
                Err(AdapterError::Aborted) => {
                    // 3a. 中止：照落收束格（`chat.assistant.final`，`status: "aborted"` +
                    // `terminated: { reason, at }`）——**思考 / 正文是事实，终止是状态**：
                    // 半截正文 / 半截思考照样入格（`text` 就是那半截），不因没说完就不记。
                    // 收束 kind 绝不复用兜底格：那是**失败**的形状（`fell_back` 专有），
                    // 落了它就等于把「用户按了停止」记成「模型答不出」，还会抬高兜底率。
                    let cost = started.elapsed().as_millis() as u64;
                    let facts = node_facts(&nodes);
                    let partial_text = facts.text.clone().unwrap_or_default();
                    if !closure_granted(&actor, ClosureKind::Speech) {
                        // 中止的收束格同过写侧闸（收束格是同一个事实，不论哪条路径
                        // 写出）：拒绝 ⇒ 不入格，该轮留在未收束态（C4 报得出）——
                        // 除非本轮落过产物格（产物收束，ADR-067）。
                        crate::plugin_warn!(
                            "actors",
                            "[v2] 中止收束被授权拒绝（{} 缺 reply.first），本轮不入格（turn={turn}）",
                            actor.principal
                        );
                        return Ok(TurnOutcome {
                            turn,
                            text: partial_text,
                            cost_ms: cost,
                            usage: None,
                            fell_back: false,
                            aborted: true,
                            awaits_user: false,
                            closure_denied: artifacts_landed == 0,
                        });
                    }
                    let produced = produced_payload(
                        &partial_text,
                        llm.model_id(),
                        &turn_root_id,
                        &facts,
                        &all_tool_calls,
                        &rounds,
                    );
                    store.append(
                        Event::pending(
                            format!("f-{turn}"),
                            EVENT_ASSISTANT_FINAL,
                            Entity::Turn,
                            Verb::Closed,
                            turn,
                            &actor.principal,
                        )
                        .with_produced_by(user_seq)
                        .with_cost_ms(cost)
                        .with_payload(with_terminated(with_status(produced, "aborted"))),
                    )?;
                    return Ok(TurnOutcome {
                        turn,
                        text: partial_text,
                        cost_ms: cost,
                        usage: None,
                        fell_back: false,
                        aborted: true,
                        awaits_user: false,
                        closure_denied: false,
                    });
                }
                Err(e) => {
                    // 3b. 兜底落格（I3：到点必答——失败也是一句话，不是静默）。
                    //     失败前已产出的半截正文 / 思考照样进载荷（它们是事实）。
                    let cost = started.elapsed().as_millis() as u64;
                    let facts = node_facts(&nodes);
                    let produced = produced_payload(
                        &facts.text.clone().unwrap_or_default(),
                        llm.model_id(),
                        &turn_root_id,
                        &facts,
                        &all_tool_calls,
                        &rounds,
                    );
                    return Self::append_fallback(
                        store,
                        &actor,
                        turn,
                        user_seq,
                        &e.to_string(),
                        cost,
                        produced,
                        artifacts_landed,
                    )
                    .await;
                }
            }
        }
    }

    /// **反射档的一轮**（[roadmap/S11 §2–§3](../../../../docs/stage/ST-011-技能编译与自我改进.md "ST-011-技能编译与自我改进")）：
    /// 技能命中 ⇒ 以技能正文收束，**一次模型调用都不发生**。
    ///
    /// ## 为什么是独立入口，而不是给 [`Self::run_with_tools`] 塞一个"不调模型"的适配器
    ///
    /// 时延闸门的全部表达在**签名**上（[`crate::symbio_core::adapters`] 模块文档）：
    /// `run_with_tools` 要 `&FullModel`，而反射档只能签出 `RuleOnly`。若为了复用那条
    /// 路径给反射档发一张 `FullModel`，「反射档不得调用模型」就从**构造期约束**退回成
    /// 一句声明——正是 `docs/plan/verify/latency_gate.rs` 要取代的那种形态（那里
    /// `assemble(Reflex)` 的产物**结构上就没有 LLM 字段**）。本函数是它的落地：
    /// **没有 `llm` 形参**，于是"反射档调模型"不是被检测到，而是写不出来。
    ///
    /// ## 为什么入参里没有 [`TurnInput`]
    ///
    /// `TurnInput` 的 `tier` / `window_turns` / `resume` 三项都服务**生成**：prompt 从
    /// 哪接、历史看多远、按哪一档装配。反射档不生成、不读 prompt，三项一个都用不上；
    /// 带上它们只会让「反射档**续写**一轮」这种语义上说不通的调用在类型上变得可以写
    /// （续写轮要跑的是刚被批准的那次工具，拿技能正文把它顶掉等于把用户批准的动作丢掉）。
    /// 故本函数只收它真正需要的：轮号、主体、用户发言、产物、出口。
    ///
    /// ## 落格与 [`Self::run_with_tools`] 同一份纪律
    ///
    /// `u-{turn}`（开轮）→ `f-{turn}`（收束，`produced_by` 指向开轮格、`cost_ms` 实测）。
    /// 开轮载荷的 `tier` 恒为 `reflex`——档位不是调用方填的字符串，而是**令牌类型**所
    /// 证明的那一档（`RuleOnly` 与 [`LatencyTier::Reflex`] 是同一个事实的两种写法，
    /// 另写一处必然漂移）。收束载荷的 `model` 也记 `reflex`：反射档没有模型，但"这段话
    /// 是谁产的"仍要可观测，空着会让命中轮与普通轮在事件面上无从区分；**命中哪一条技能**
    /// 则由触发串与技能事件唯一确定，不在这里复述一遍（能算出来的不占字段）。
    ///
    /// 产物经 `sink` 上线——与真实路径**同一个出口**。不上线，收束节点在前端就永远建不
    /// 起来（`chat_loop` 用 `response_text_child_id` 定格本轮正文），表现为"答了但看不见"。
    ///
    /// ## 为什么参数多到要 `allow`
    ///
    /// 七个参数**各自是一个不同的端口**（事实源 / 令牌 / 轮号 / 主体 / 用户发言 / 产物 /
    /// 出口），与 [`Self::run_with_tools`] 同款理由：打包成一个结构体只是把"七个端口"
    /// 改名叫"一个结构体 + 七个字段"，调用方仍要逐个填。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_reflex<S>(
        &self,
        store: &S,
        _tok: &crate::symbio_core::RuleOnly,
        turn: u64,
        actor: &ActorSpec,
        utterance: &str,
        text: &str,
        sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
    ) -> Result<TurnOutcome, crate::symbio_core::AppendError>
    where
        S: Store<Event = Event>,
    {
        let started = std::time::Instant::now();
        let user_seq = store
            .append(
                Event::pending(
                    format!("u-{turn}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    turn,
                    "user",
                )
                .with_payload(serde_json::json!({
                    "text": utterance,
                    "tier": LatencyTier::Reflex.name(),
                })),
            )
            .map(|seq| seq.value())?;
        sink.on_delta(text);
        let cost_ms = started.elapsed().as_millis() as u64;
        if !closure_granted(actor, ClosureKind::Speech) {
            // 与深度档同一条写侧闸（见 [`closure_granted`]）：拒绝 ⇒ 不落收束格。
            crate::plugin_warn!(
                "actors",
                "[v2] 反射档收束被授权拒绝（{} 缺 reply.first），本轮不入格（turn={turn}）",
                actor.principal
            );
            return Ok(TurnOutcome {
                turn,
                text: text.to_string(),
                cost_ms,
                usage: None,
                fell_back: false,
                aborted: false,
                awaits_user: false,
                closure_denied: true,
            });
        }
        store.append(
            Event::pending(
                format!("f-{turn}"),
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                turn,
                &actor.principal,
            )
            .with_produced_by(user_seq)
            .with_cost_ms(cost_ms)
            // 与深度档同一个产出形状（`produced_payload`）+ 状态字段：反射档无模型、
            // 无流式节点，`turn_id` 记空串（重建侧按事件 id 稳定合成）、取数口不挂。
            // 载荷形状不另写一份——重建只按一个形状读。
            .with_payload(with_status(
                produced_payload(
                    text,
                    LatencyTier::Reflex.name(),
                    "",
                    &StreamNodeFacts::default(),
                    &[],
                    &[],
                ),
                "completed",
            )),
        )?;
        Ok(TurnOutcome {
            turn,
            text: text.to_string(),
            cost_ms,
            usage: None,
            fell_back: false,
            aborted: false,
            awaits_user: false,
            closure_denied: false,
        })
    }

    /// 兜底落格（I3）的唯一构造点：失败也是一句话。
    ///
    /// 载荷 = **产出部分**（`produced`，失败前已产出的半截正文 / 思考 / 工具事实——
    /// 它们是事实，不因失败就不记）+ `why`（失败原因）+ `status: "failed"`（失败是
    /// 状态，不进产出形状本身）。
    ///
    /// `artifacts_landed` = 本轮已新落的产物格数（ADR-067 的产物收束判据）：兜底格
    /// 本身也是发言收束，发言闸拒它时，本轮**落过产物**就不算未收束——
    /// `closure_denied` 为假，派生事实照写（失败呈现照旧：`fell_back` 不变）。
    // 参数全是同一轮收束的平铺事实（主体 / 轮次 / 锚 / 原因 / 计时 / 产出 / 产物计数），
    // 彼此独立、无分组语义——拆结构体反而隔一层（照 model/stream.rs 同款先例）。
    #[allow(clippy::too_many_arguments)]
    async fn append_fallback<S>(
        store: &S,
        actor: &ActorSpec,
        turn: u64,
        user_seq: u64,
        why: &str,
        cost_ms: u64,
        produced: serde_json::Value,
        artifacts_landed: u64,
    ) -> Result<TurnOutcome, crate::symbio_core::AppendError>
    where
        S: Store<Event = Event>,
    {
        if !closure_granted(actor, ClosureKind::Speech) {
            // 兜底格也是**收束格**，同过写侧闸（见 [`closure_granted`]）：拒绝 ⇒ 不落格。
            // `fell_back` 仍为真——模型确实失败了（I3 的「到点必答」由调用方按失败呈现）。
            crate::plugin_warn!(
                "actors",
                "[v2] 兜底收束被授权拒绝（{} 缺 reply.first），本轮不入格（turn={turn}）",
                actor.principal
            );
            return Ok(TurnOutcome {
                turn,
                text: why.to_string(),
                cost_ms,
                usage: None,
                fell_back: true,
                aborted: false,
                awaits_user: false,
                closure_denied: artifacts_landed == 0,
            });
        }
        let mut payload = with_status(produced, "failed");
        payload["why"] = serde_json::Value::String(why.to_string());
        store.append(
            Event::pending(
                format!("fb-{turn}"),
                EVENT_ASSISTANT_FALLBACK,
                Entity::Turn,
                Verb::Closed,
                turn,
                &actor.principal,
            )
            .with_produced_by(user_seq)
            .with_cost_ms(cost_ms)
            .with_payload(payload),
        )?;
        Ok(TurnOutcome {
            turn,
            text: why.to_string(),
            cost_ms,
            usage: None,
            fell_back: true,
            aborted: false,
            awaits_user: false,
            closure_denied: false,
        })
    }

    /// 中止收束格的构造点（ADR-052 决策 1）：**闸门与首请求之间**到达的中止。
    ///
    /// 与 adatper 中途返回 `AdapterError::Aborted` 落的是**同一格**
    /// （`chat.assistant.final` + `status: "aborted"` + `terminated`）——终止是事实的
    /// **状态**，不是新种类（ADR-052 决策 3 明令「不为中止 / 等待新增事实 kind」）。
    /// 两处落格**共用形状**：本函数与 3a 分支构造的载荷逐字一致，只差半截产出为空
    /// （此时还没发请求，模型什么都没产出）。
    ///
    /// 为什么它必须存在而不是让调用方自己落格：`user.message` 已由本运行器在本函数
    /// 之前落了（1 步），**收束格与本轮用户格配对**（N3/C4）的不变量归本运行器守，
    /// 插件侧不得自己补写那两格（见文件头「落格纪律」）。中止轮若由插件自行收束，
    /// 就绕过了这道闸。
    async fn append_aborted<S>(
        store: &S,
        actor: &ActorSpec,
        turn: u64,
        user_seq: u64,
        llm: &dyn LlmAdapter,
        turn_root_id: &str,
    ) -> Result<TurnOutcome, crate::symbio_core::AppendError>
    where
        S: Store<Event = Event>,
    {
        if !closure_granted(actor, ClosureKind::Speech) {
            // 中止的收束格同过写侧闸（与 3a 分支同一条纪律）：拒绝 ⇒ 不入格，
            // 该轮留在未收束态（C4 报得出）。
            crate::plugin_warn!(
                "actors",
                "[v2] 中止收束被授权拒绝（{} 缺 reply.first），本轮不入格（turn={turn}）",
                actor.principal
            );
            return Ok(TurnOutcome {
                turn,
                text: String::new(),
                cost_ms: 0,
                usage: None,
                fell_back: false,
                aborted: true,
                awaits_user: false,
                closure_denied: true,
            });
        }
        // 半截产出为空：请求还没发出，模型没产出任何正文 / 思考 / 工具事实。
        let produced = produced_payload(
            "",
            llm.model_id(),
            turn_root_id,
            &StreamNodeFacts::default(),
            &[],
            &[],
        );
        store.append(
            Event::pending(
                format!("f-{turn}"),
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                turn,
                &actor.principal,
            )
            .with_produced_by(user_seq)
            .with_cost_ms(0)
            .with_payload(with_terminated(with_status(produced, "aborted"))),
        )?;
        Ok(TurnOutcome {
            turn,
            text: String::new(),
            cost_ms: 0,
            usage: None,
            fell_back: false,
            aborted: true,
            awaits_user: false,
            closure_denied: false,
        })
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
/// [ADR-065]: ../../../../docs/adr/ADR-065.md "ADR-065"
/// [ADR-067]: ../../../../docs/adr/ADR-067.md "ADR-067"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClosureKind {
    /// 发言收束：答话 / 兜底 / 中止的那一格。
    Speech,
    /// 产物收束：工具结果的那一格（`artifact.added`）。
    Artifact,
}

/// 写侧授权闸（[plan/01 §7](../../../../docs/plan/PL-001-核心架构.md "PL-001-核心架构") 写侧，
/// [04 §3.1 批⑥](../../../../docs/plan/PL-004-工程落地.md "PL-004-工程落地")）：该主体此刻能否写
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
/// 拒绝 ⇒ **不落格**：
/// - 发言臂 ⇒ 该轮留在未收束态，`check_all` 会把它报出来（C4）——**除非本轮落过
///   产物格**，那同样算收束（`unresolved_turns` 与产物闸同批认可）；
/// - 产物臂 ⇒ 那一条工具结果不入格（warn 后跳过）。
///
/// 平凡值下（`agent:main`，以及任何经 `matrix_for` 派生的 `agent:<id>`）两臂恒
/// 放行——闸是 fail-closed 的**结构**，不是生产里会翻面的开关。
fn closure_granted(actor: &ActorSpec, kind: ClosureKind) -> bool {
    match kind {
        // `true` = 该轮尚无收束格（八个调用点全是首条收束）。「轮次位置 → 能力」的
        // 映射住 core（`Capability::reply_capability`），规格方法与表谓词共用它。
        ClosureKind::Speech => actor.can_land_speech(true),
        ClosureKind::Artifact => actor.can_land_artifact(),
    }
}

/// 一轮工具交换 → **结构化**消息（ADR-048a）。
///
/// 出的是真消息：assistant（带 `tool_calls` 摘要）/ tool（带 `tool_call_id`）。
/// 拍成散文（`助手请求工具: … / 工具结果(x): …`）会同时坏三件事——把工具调用当成
/// **assistant 说的话**、把结果做成**没有角色的文本**、**丢掉 `tool_call_id`**
/// （provider 侧协议要求 `tool` 消息关联到具体那次调用，否则多工具并发时无法配对）。
///
/// ## `round` 参数的存在理由：这一批消息要**跨轮累加**
///
/// 交换是**累加**进下一次请求的（见运行器 2e：模型必须看见本回合此前每一次调用与
/// 结果，否则重复调用同一个工具）。于是 id 的唯一性域也从「一次请求」扩到「整个
/// 回合」——同一工具在两个工具轮各调一次，若两侧的 id 相同，adapter 的 `last_result_of`
/// 会把它当成**同一次调用的两条结果**而只留最后那条（第一次的结果凭空消失）。
/// id 因此优先取调用自身带得的身份（wire id / 节点 id，二者天然跨轮唯一），
/// 两者都缺才合成，且合成式里带上轮次（见 [`prompt_call_id`]）。
fn tool_exchange_messages(
    turn: &crate::symbio_core::LlmTurn,
    outcomes: &[crate::symbio_core::DispatchOutcome],
    round: usize,
) -> Vec<crate::symbio_core::PromptMessage> {
    let mut out: Vec<crate::symbio_core::PromptMessage> = Vec::new();
    if !turn.text.trim().is_empty() {
        out.push(crate::symbio_core::PromptMessage {
            role: "assistant".into(),
            text: turn.text.trim().to_string(),
            tool_call_id: None,
            tool: None,
            tool_calls: None,
        });
    }
    // 调用请求：作为 assistant 消息，**带真正的 `tool_calls` 结构**。
    //
    // ⚠️ `tool_calls` 字段不能省（实测踩过）：协议层的请求包清洗会丢弃「无对应
    // `tool_call` 的 tool 结果」，而「对应的 tool_call」是从 assistant 消息的
    // `tool_calls` **结构**里认的，不是从正文里认的。只在正文写「调用工具 x」的话，
    // 后面每一条 tool 结果都成孤儿被丢 ⇒ 模型永远收不到结果 ⇒ **无限工具循环**
    // （实测 9905 次请求、CLI 撞 120s 超时，症状完全指不到这一层）。
    //
    // 正文那行仍然保留：它是给人读的（诊断 / 日志 / 断言），不是给协议用的。
    for (i, call) in turn.tool_calls.iter().enumerate() {
        let name = call.name.as_deref().unwrap_or("<unnamed>");
        let id = prompt_call_id(call, round, i);
        out.push(crate::symbio_core::PromptMessage {
            role: "assistant".into(),
            text: format!("调用工具 {} {}", name, call.arguments),
            tool_call_id: Some(id.clone()),
            tool: Some(name.to_string()),
            tool_calls: Some(vec![PromptToolCall {
                id,
                name: name.to_string(),
                arguments: call.arguments.clone(),
            }]),
        });
    }
    // 结果：逐条 tool 消息，**按调用配对**取 id（不是按工具名）。
    //
    // ⚠️ 为什么不能按名字配：同一次响应里调用同一个工具两次时，按名配对会把**两条结果
    // 都挂到第一次调用**的 id 上。而 adapter 侧 `last_result_of` 按 id 取「同一次调用的
    // 最终那条结果」——两条同 id 的结果在模型眼里就是一次调用的两条，第一次的真实
    // 结果被吃掉，第二次拿到别人的正文。**结果与调用错配比缺一结果更坏**：模型据此
    // 改道，而它改的是凭据不足的道。
    //
    // 配不上调用时（无对照物）退回结果自己的 `call_id`：它在本次请求里没有对应的
    // 调用声明，adapter 会把它降级成上下文（`declared_here` 为假那条路）。
    for outcome in outcomes {
        let at = turn
            .tool_calls
            .iter()
            .position(|c| outcome_of(outcomes, c).is_some_and(|o| o.call_id == outcome.call_id));
        let id = match at {
            Some(i) => prompt_call_id(&turn.tool_calls[i], round, i),
            None => outcome.call_id.clone(),
        };
        out.push(crate::symbio_core::PromptMessage {
            role: "tool".into(),
            text: outcome.text.clone(),
            tool_call_id: Some(id),
            tool: Some(outcome.name.clone()),
            tool_calls: None,
        });
    }
    out
}

/// 续写轮的恢复交换 → 结构化消息。与 [`tool_exchange_messages`] 同形：
/// 「这一轮是续写」是**调度事实**，不该变成模型眼里的另一种协议。
fn exchange_messages(
    call: &crate::symbio_core::TurnToolCallInfo,
    text: &str,
) -> Vec<crate::symbio_core::PromptMessage> {
    let name = call.name.as_deref().unwrap_or("<unnamed>");
    // 恢复交换只有这一次调用 ⇒ 轮次记 0（id 仍优先取调用自带的身份，见
    // [`prompt_call_id`]）：恢复的调用与本轮其它调用共属同一次请求，撞名即错配。
    let id = prompt_call_id(call, 0, 0);
    vec![
        crate::symbio_core::PromptMessage {
            role: "assistant".into(),
            text: format!("调用工具 {} {}", name, call.arguments),
            tool_call_id: Some(id.clone()),
            tool: Some(name.to_string()),
            tool_calls: Some(vec![PromptToolCall {
                id: id.clone(),
                name: name.to_string(),
                arguments: call.arguments.clone(),
            }]),
        },
        crate::symbio_core::PromptMessage {
            role: "tool".into(),
            text: text.to_string(),
            tool_call_id: Some(id),
            tool: Some(name.to_string()),
            tool_calls: None,
        },
    ]
}

/// 合成 / 选取 `tool_call_id`（跨轮唯一，见 [`tool_exchange_messages`] 的 `round`）。
///
/// 优先级与 v1 的请求构建**同一口径**（`plugins/model/message_builder.rs`：wire id
/// 优先、历史数据回退节点 id）：
/// 1. `wire_id`——provider 自己签发的 `tool_call_id`，模型认得它，跨轮天然唯一；
/// 2. `id`——节点 id（会话内唯一），provider 没给 wire id 时的对照物；
/// 3. 合成 `call_<tool>_<轮>_<序号>`——两者都缺（桩 / 老数据）时兜底，
///    **带轮次**是为了跨轮累加后不与任何一次调用撞名。
///
/// 前缀 `call_` 是**故意的**：它让 id 一眼可辨是合成的（provider 侧回填时若
/// 与真实调用 id 空间混淆，冲突能被看见而不是静默）。
fn prompt_call_id(
    call: &crate::symbio_core::TurnToolCallInfo,
    round: usize,
    index: usize,
) -> String {
    fn present(id: &Option<String>) -> Option<String> {
        id.clone().filter(|s| !s.trim().is_empty())
    }
    present(&call.wire_id)
        .or_else(|| present(&call.id))
        .unwrap_or_else(|| {
            synthetic_call_id(call.name.as_deref().unwrap_or("<unnamed>"), round, index)
        })
}

fn synthetic_call_id(tool: &str, round: usize, index: usize) -> String {
    format!("call_{tool}_{round}_{index}")
}

#[cfg(test)]
#[path = "turn_runner.test.rs"]
mod tests;
