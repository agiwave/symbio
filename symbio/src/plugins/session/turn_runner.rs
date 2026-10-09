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
//! `grep -c TurnRunner docs/plan/01-核心架构.md` = 0。真正的冻结锚点
//! （[plan/03 §1](../../../../docs/plan/03-演进与验证.md)）是：F1 `Store` 签名 /
//! F2 `Projection` 构造形状 / **F3 `ActorSpec` 字段集 + 档位→令牌映射** /
//! F4 三条不变量 / F5 三个封闭集合 / F6 参数表键集。**`TurnRunner` 不在其中**，
//! 所以这次下沉**不改 F1–F6**，按 [plan/03 §2](./../../../docs/plan/03-演进与验证.md)
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
    EVENT_USER_MESSAGE,
};
use std::borrow::Cow;

// ── v2 会话运行时：turn 运行器（chat_loop 切换的第一块可复用件）────────
//
// 职责把 [plan/04 §2](../../../../docs/plan/04-工程落地.md) 的单轮流程落成
// 一个函数调用：用户消息入格 → [`Reasoner`] 生成（实测耗时）→ final / fallback
// 落格（I3：失败也必须有输出）。不变量靠构造成立：每轮独立 turn 号（N3）、
// final 溯源指向本轮用户消息（N5）、实测 cost_ms 随事件入账（ADR-044）。
// 未来 chat_loop 切到 v2 链路时复用本运行器；当前由真实端点校准彩排使用。

/// 一轮的**调度输入**（full 档会话轮与彩排共用的入参包）。
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
    /// `viewer` 都取它（[plan/11 批 1](../../../../docs/plan/11-多执行器与多主体加固实施方案.md)
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
    /// `full` 档的 prompt 是**一条** user 消息（`ProviderLlmAdapter::generate_turn`
    /// 把渲染结果整段发出），而 v1 的请求视图层
    /// （`build_request_view`）产出的长期记忆召回段 / 就绪任务集段 / 委派者真源段
    /// 是**独立的消息**。于是「档位翻成 `full`」的同时，这三段**静默**从模型眼前消失
    /// ——事实照样入格、`session/stats` 照样有数，只有模型看不见（实测 t29 / t35
    /// 在 `full` 下报「记忆段没注入」，而注入逻辑一行没改）。
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
    /// 用户在助手干活途中补了一句话。在 v1 路径上它被折进 `context.messages`
    /// ——但 `full` 档走的是 `v2_exec` → `TurnRunner`，而 `v2_exec` 里**根本没有
    /// 抽干补充的代码**（v1 那段被档位分叉绕过了）。于是：
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
    /// `classify` 判 `Answered` ⇒ `compose` 出好答话 ⇒ 本仓这一轮**一个模型请求都不该
    /// 发**。v1 的做法是把这个答话 push 进 `context.messages` 后收尾；而 `full` 档的
    /// prompt 从事实网格投影 ⇒ 那次问答在网格里**完全不存在**（用户格与收束格都由
    /// 本运行器落，而该轮根本不进运行器）⇒ 下一轮模型不知道自己刚回答过。
    ///
    /// ## 为什么由 core 承接而不是插件自己落那两格
    ///
    /// 「一个 turn = 用户格 opened + 至多一条收束 closed」这条不变量由运行器守着。
    /// 插件自己补写就成了**第二个轮次事实写方**——那正是 `chat_loop` 的
    /// `!turn.v2_executed` 拦截要防的形状（同轮两份记账的假象）。所以走这里：
    /// 答话给运行器，格仍由它落。
    ///
    /// ## `model` 载荷字段留空
    ///
    /// 这一轮没有模型参与，`chat.assistant.final` 的 `model` 载荷记**空串**而不是
    /// 任何模型的 id——记一个会变成「这次请求用的是那个模型」，而事实相反。
    pub final_reply: Option<String>,
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
            .field("tier", &self.tier)
            .field("window_turns", &self.window_turns)
            .field("resume", &self.resume)
            .field("actor", &self.actor)
            .field("prefix", &self.prefix)
            .field("final_reply", &self.final_reply)
            // 闭包**不展开**：展开会把捕获的东西（生产里是用户对话正文）打进日志。
            .field("inject", &self.inject.as_ref().map(|_| "<闭包>"))
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
    /// 中止（用户主动停止 / 会话销毁）：**既无 final 也无兜底格**——网格只留
    /// 已入格的用户消息，少一格是诚实的缺口（ADR-044 的转写纪律同源）；
    /// `text` 为空。与 `fell_back` 互斥（失败要落兜底格，中止不落），因此
    /// **绝不进兜底分子**（兜底率是失败的指标）。
    pub aborted: bool,
    /// 本轮**收束于等待用户动作**（工具报了 `failure_kind = pending`，如 confirm /
    /// ask_user）：运行器停止工具循环并**不落收束格**——本轮尚未了结，等用户答完
    /// 才续（恢复是 [plan/11 批 2](../../../../docs/plan/11-多执行器与多主体加固实施方案.md) 的
    /// 能力）。与 `aborted` 同样是「网格少一格的诚实缺口」，但原因不同：一个是被放弃，
    /// 一个是还没完。
    pub awaits_user: bool,
    /// 本轮收束**被写侧闸拒绝**（主体不持 `reply.first`，见 [`closure_granted`]）：
    /// 收束格未入格，该轮留在未收束态（`check_all` 的 C4 报得出）。与 `aborted` /
    /// `awaits_user` 同属「网格少一格」，但**原因在授权**，不是放弃也不是等待——
    /// 调用方据此**跳过本轮的派生事实**（与桥档 `authorize_close` 拒绝时同形：
    /// 那一档的 `record_to_wal` 在闸处提前返回，承诺 / 任务 / 熔断 / 记忆全不写）。
    ///
    /// 平凡值 `false`：生产里主体恒为 `agent:main` 或经 `matrix_for` 派生的
    /// `agent:<id>`，都持 `reply.*` ⇒ 闸是 fail-closed 的**结构**，不是会翻面的开关。
    pub closure_denied: bool,
}

/// 按主体过滤一批事件（**可见域入口**，[plan/11 批 1](../../../../docs/plan/11-多执行器与多主体加固实施方案.md)
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
fn window_by_turn<'a>(
    events: &'a [Event],
    current_turn: u64,
    keep: u64,
    viewer: Option<&str>,
) -> Cow<'a, [Event]> {
    let windowed = if keep == 0 {
        // `keep == 0` = **不截断**（全量）——与 `SessionConfig::context_messages`
        // 的 `0` 同一条口径（那里 `0` 一直是「上下文窗口不按轮次截断」）。两条执行路径
        // 对同一个数必须同义：否则出厂 `full` 档下配 `0` 的实例会**静默**丢掉全部历史
        // ——事实照样入格、`session/stats` 照样有数，只有模型看不见。
        events
    } else {
        let oldest = current_turn.saturating_sub(keep - 1);
        let cut = events
            .iter()
            .position(|e| e.entity == Entity::Turn && e.turn >= oldest)
            .unwrap_or(events.len());
        &events[cut..]
    };
    filter_visible(windowed, viewer)
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
    /// 工具结果——[plan/10 批 3](../../../../docs/plan/10-工具轮v2化实施方案.md) 起）；
    /// 而**本轮**刚发生的交换，投影里当然还没有（快照取在它落格之前），故由本函数
    /// **就地累积**追加——模型必须看到自己请求过什么、拿到了什么，否则会重复调用
    /// 同一个工具。
    ///
    /// ## 中止 / 等待用户
    ///
    /// 两者都**不落收束格**（网格少一格是诚实缺口，ADR-044 同源），差别在原因：
    /// 中止是被放弃（`aborted`），等待用户是还没完（`awaits_user`，
    /// [plan/11 批 2](../../../../docs/plan/11-多执行器与多主体加固实施方案.md) 续跑）。
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
        } = input;
        // 本轮的 viewer：历史窗口只交**本主体看得见**的事件（plan/11 批1 ③）。
        // 身份是入参不是字面量——`actor.principal` 由调用方（生产：会话属于哪个
        // agent）给出；S08 §4 平凡值 `agent:main` 下没有任何事件被滤掉。
        let viewer = Some(actor.principal.as_str());
        // 1. 用户消息入格（turn × opened），档位随载荷入账。
        //
        // 续写轮（`resume`）**不新开用户格**：用户没再说话，重开会让同一句话在网格里
        // 出现两次、把那一轮数成两轮；且 C4 按 turn 号配对——续写轮要的正是让**原轮**
        // 收到收束事件（见 [`TurnResume`]）。此时复用该轮既有的用户格 seq 作溯源锚点。
        let user_seq = match &resume {
            None => store
                .append(
                    Event::pending(
                        format!("u-{turn}"),
                        EVENT_USER_MESSAGE,
                        Entity::Turn,
                        Verb::Opened,
                        turn,
                        "user",
                    )
                    .with_payload(serde_json::json!({ "text": text, "tier": tier.name() })),
                )
                .map(|seq| seq.value())?,
            Some(r) => r.user_seq,
        };
        // 1b. 定稿答话（缺口 5）：用户格照常开，答话照常收束，**模型不参与**。
        //
        // 为什么放在这里而不是循环里：这一轮一次请求都不发，循环的第一件事
        // （渲染 prompt、调模型）对它全都是白工。放在用户格之后是因为收束格的
        // 溯源必须指向本轮用户格（I2）——而那只有落完用户格才有。
        //
        // 空答话与模型返回空文本同一条纪律：不落收束，落到兜底话术上。把空串
        // 记成「答了空话」会让「到点必答」这条线无声失效（I3）。
        if let Some(reply) = final_reply.as_deref() {
            if reply.trim().is_empty() {
                // 耗时记 0：这一轮一次模型请求都没发（答话是调用方给好的
                // `final_reply`），`started` 那条计时口径本来就是「模型往返」。
                return self
                    .append_fallback(store, &actor, turn, user_seq, "compose returned empty", 0)
                    .await;
            }
            if !closure_granted(&actor.principal) {
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
                .with_cost_ms(0)
                .with_payload(serde_json::json!({
                    "text": reply,
                    // 空串：这轮没有模型参与（见 `TurnInput::final_reply` 的载荷说明）
                    "model": "",
                })),
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
        // 实测耗时：跨轮累加（一次用户轮可能有多次 LLM 请求，ADR-044 的实测口径）。
        let mut cost_ms = 0u64;
        // 产物格的事件 id 需要在本轮内唯一（同一工具可被调用多次）。续写轮从**该轮
        // 已有产物数**起编号——等待轮可能已落过产物格，从 0 起会撞幂等键（`Duplicate`）。
        let mut artifact_no = full_snapshot
            .iter()
            .filter(|e| e.turn == turn && e.kind == crate::symbio_core::EVENT_ARTIFACT_ADDED)
            .count() as u64;

        // 续写轮：恢复产生的工具交换是**本轮已发生的事实**——先落格（`artifact.added`，
        // 溯源指向本轮用户格，I2）再拼进 prompt。落格之后它固然进了投影，但本轮的
        // `base_prompt` 已在上方渲染完毕（快照早于这次 append），故仍就地追加——模型
        // 必须看到「我请求了什么、拿到了什么」，否则会重复调用同一个工具。
        if let Some(r) = &resume {
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
                })),
            )?;
            artifact_no += 1;
            exchange.extend(exchange_messages(&r.call, &r.text));
        }

        loop {
            // 基线 + 本轮交换。**拼接消息数组**：拼字符串会把角色拍平、丢掉 `tool_call_id`。
            let mut messages = base_messages.clone();
            messages.extend(exchange.iter().cloned());

            // 2. 生成（实测耗时在 adapter 边界取得；失败路径的耗时从调用起点算）。
            match llm.generate_turn(tok, &messages, tools, sink.clone()).await {
                Ok(rt) => {
                    cost_ms += rt.cost_ms;

                    // 2a. 本轮不再请求工具 ⇒ 收束（落 final）。
                    if rt.tool_calls.is_empty() {
                        if rt.text.trim().is_empty() {
                            // 空文本 + 无工具调用 = 模型什么也没答。这是**失败**：
                            // 落成 final 会把「没答」记成「答了空话」（I3 的到点必答
                            // 要求有一句话，兜底话术承担它）。
                            let cost = started.elapsed().as_millis() as u64;
                            return self
                                .append_fallback(
                                    store,
                                    &actor,
                                    turn,
                                    user_seq,
                                    "model returned empty text",
                                    cost,
                                )
                                .await;
                        }
                        if !closure_granted(&actor.principal) {
                            // 写侧闸拒：**不落收束格**——该轮留在未收束态，`check_all`
                            // 会把它报出来（C4），与桥档 `authorize_close` 同一条纪律、
                            // 同一个谓词。用户的答案照旧可见（`text` 仍回给调用方）。
                            crate::plugin_warn!(
                                "actors",
                                "[v2] 收束被授权拒绝（{} 缺 reply.first），本轮不入格（turn={turn}）",
                                actor.principal
                            );
                            return Ok(TurnOutcome {
                                turn,
                                text: rt.text,
                                cost_ms,
                                usage: rt.usage,
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
                            .with_payload(serde_json::json!({
                                "text": rt.text,
                                "model": llm.model_id(),
                            })),
                        )?;
                        return Ok(TurnOutcome {
                            turn,
                            text: rt.text,
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
                        return self
                            .append_fallback(
                                store,
                                &actor,
                                turn,
                                user_seq,
                                "模型请求了工具，但本轮没有工具分发通道",
                                cost,
                            )
                            .await;
                    };
                    let outcomes = dispatch.dispatch(&rt).await;

                    // 2c. 产物落格：每条工具结果一格，溯源指向本轮用户格。
                    for outcome in &outcomes {
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
                            })),
                        )?;
                        artifact_no += 1;
                    }

                    // 2d. 收束于等待用户 ⇒ 停止循环且**不落收束格**（见函数文档）。
                    if outcomes.iter().any(|o| o.needs_user_action) {
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

                    // 2e. 把这一轮交换追加进 prompt，继续下一轮。
                    exchange.clear();
                    exchange.extend(tool_exchange_messages(&rt, &outcomes));

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
                    // 3a. 中止：**不落任何收束格**——用户消息已入格，少一格是诚实
                    // 的缺口（ADR-044 同一纪律）。兜底格是**失败**的形状（`fell_back`
                    // 专有），中止落了它就等于把「用户按了停止」记成「模型答不出」，
                    // 还会抬高兜底率——那是假象。
                    let cost = started.elapsed().as_millis() as u64;
                    return Ok(TurnOutcome {
                        turn,
                        text: String::new(),
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
                    let cost = started.elapsed().as_millis() as u64;
                    return self
                        .append_fallback(store, &actor, turn, user_seq, &e.to_string(), cost)
                        .await;
                }
            }
        }
    }

    /// **反射档的一轮**（[roadmap/S11 §2–§3](../../../../docs/plan/roadmap/S11-技能编译与自我改进.md)）：
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
        if !closure_granted(&actor.principal) {
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
            .with_payload(serde_json::json!({
                "text": text,
                "model": LatencyTier::Reflex.name(),
            })),
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
    async fn append_fallback<S>(
        &self,
        store: &S,
        actor: &ActorSpec,
        turn: u64,
        user_seq: u64,
        why: &str,
        cost_ms: u64,
    ) -> Result<TurnOutcome, crate::symbio_core::AppendError>
    where
        S: Store<Event = Event>,
    {
        if !closure_granted(&actor.principal) {
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
                closure_denied: true,
            });
        }
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
            .with_payload(serde_json::json!({ "why": why })),
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
}

/// 写侧授权闸（[plan/01 §7](../../../../docs/plan/01-核心架构.md) 写侧，
/// [04 §3.1 批⑥](../../../../docs/plan/04-工程落地.md)）：该主体此刻能否写本轮的
/// **收束格**。
///
/// ## 为什么运行器也要判（它原先只在桥档判）
///
/// 收束格的写方**跟着执行路径走**：`bridge` 档由 `v2_facts::record_to_wal` 落格
/// （那里有 `authorize_close`），`full` 档由**本运行器**原生落格。闸只挂在其中一条
/// 路径上，另一条就整条漏判——而**没有任何东西会变红**（两条路径各写各的收束格，
/// 谁也不看谁）。这与 S12 那批查出的「文档断言了、生产数据里却相反」是同一类缺口。
///
/// 三条落格路径（深度档 final / 反射档 final / 兜底格）都过这里——收束格是**同一个
/// 事实**，不论它由哪条路径、哪种收束形态写出。
///
/// ## 谓词与判据
///
/// 谓词是 `PermissionMatrix::can_reply`（桥档 `authorize_close` 用的也是它——写侧闸
/// 判什么**只定义一次**）。`first` 恒为 `true`：运行器一轮只落一格收束
/// （`final_unique_per_turn` 是不变量），不存在「同轮追加」那一态——那是 v1 重试的
/// 形状，只有桥档才有。
///
/// 拒绝 ⇒ **不落格**：该轮留在未收束态，`check_all` 会把它报出来（C4），与桥档同一
/// 条纪律。平凡值下（`agent:main`，以及任何经 `matrix_for` 派生的 `agent:<id>`）恒
/// 放行——闸是 fail-closed 的**结构**，不是生产里会翻面的开关。
fn closure_granted(principal: &str) -> bool {
    crate::symbio_core::authz::matrix_for(principal).can_reply(principal, true)
}

/// 一轮工具交换 → **结构化**消息（ADR-048a）。
///
/// 出的是真消息：assistant（带 `tool_calls` 摘要）/ tool（带 `tool_call_id`）。
/// 拍成散文（`助手请求工具: … / 工具结果(x): …`）会同时坏三件事——把工具调用当成
/// **assistant 说的话**、把结果做成**没有角色的文本**、**丢掉 `tool_call_id`**
/// （provider 侧协议要求 `tool` 消息关联到具体那次调用，否则多工具并发时无法配对）。
///
/// ## `tool_call_id` 怎么来（**已定的合成，不是缺失**）
///
/// 网格里的 `artifact.added` 只记了工具名与正文——**那才是事实**，没有调用 id
/// （调用 id 是本次运行的句柄，不是跨轮事实）。所以这里按「工具名 + 本轮序号」
/// 合成一个**稳定 id**（见 [`synthetic_call_id`]），而不是让消息裸奔：
/// 裸奔会被「拒绝未知 tool_call_id」的 provider 整条拒绝，那**比拍平更糟**
/// （拍平至少还能答）。
fn tool_exchange_messages(
    turn: &crate::symbio_core::LlmTurn,
    outcomes: &[crate::symbio_core::DispatchOutcome],
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
        let id = synthetic_call_id(name, i);
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
    // 结果：逐条 tool 消息，带上**对应那次调用**的合成 id。
    //
    // 按工具名配对（`DispatchOutcome` 不带调用 id，见其文档：core 不认识
    // `ChatMessage` 的图语义）。同名多次调用时退化为「都关联第一条」——
    // 记在 TODO 里而不是假装能配准：**配不准时多给信息好过给错的关联**，
    // 而当前的 `artifact.added` 事实里确实没有足以配准的信息。
    for outcome in outcomes {
        let same_named = turn
            .tool_calls
            .iter()
            .position(|c| c.name.as_deref() == Some(outcome.name.as_str()));
        out.push(crate::symbio_core::PromptMessage {
            role: "tool".into(),
            text: outcome.text.clone(),
            tool_call_id: Some(synthetic_call_id(
                &outcome.name,
                same_named.unwrap_or(usize::MAX),
            )),
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
    let id = synthetic_call_id(name, 0);
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

/// 合成的 `tool_call_id`：`call_<tool>_<序号>`。
///
/// 前缀 `call_` 是**故意的**：它让 id 一眼可辨是合成的（provider 侧回填时若
/// 与真实调用 id 空间混淆，冲突能被看见而不是静默）。
fn synthetic_call_id(tool: &str, index: usize) -> String {
    format!("call_{tool}_{index}")
}

#[cfg(test)]
#[path = "turn_runner.test.rs"]
mod tests;
