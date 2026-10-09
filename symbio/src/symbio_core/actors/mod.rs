//! `actors` —— 主体（v2 ②，[plan/01 §4](../../../../docs/plan/01-核心架构.md)）。
//!
//! ## 形状（01 §4 的五字段，一字未增）
//!
//! `model` 与 `context` **不是字段**：`model` 是 `pattern` 的函数（Decider→无、
//! Reasoner→LLM、Translator→小模型），`context` 是 `projection` 的参数——
//! 能由别的东西算出来的就不占字段（J1）。
//!
//! ## 依赖纪律（[plan/05 §3.1](../../../../docs/plan/05-模块架构.md) ② 行）
//!
//! 只接收**类型化输入**（事件切片 / `View`），**产出事件**——不持有任何其他模块的
//! 句柄。「想调用也拿不到对方的句柄」是构造保证，不是约定。
//!
//! ## S1 彩排（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 3 步：零 LLM 彩排）
//!
//! 闭环彩排跑在 ⑤ `adapters` 的确定性桩上（零 LLM、毫秒级），链路一条不少：
//! 用户消息入库 → 应答 → final/fallback 入库 → 三查。规则应答的语义由
//! [`classify`(../../../../docs/plan/06-会话响应性落地.md)] 的规则表以更严纪律承接，
//! 本域只留**模式名**；`Reasoner` / `Translator` 的**产出路径**在 S2 / S5 接入；
//! `capabilities` 暂为字符串数据（能力名），枚举闭集与 grants 校验归 S3 ⑥
//! `governance`——本域不抢。
//!
//! ## 名字的两处出处
//!
//! - `ActorSpec` / `Pattern` / `Scope`：[plan/01 §4](../../../../docs/plan/01-核心架构.md)
//!   的冻结契约名（名字先于模块存在，判据同 `schemas`）；
//! - [`Pattern::Decider`]：[plan/05 §4](../../../../docs/plan/05-模块架构.md) S8 的
//!   反射档判定者——模式名在此，规则应答不在（见上）。

use crate::symbio_core::adapters::{AdapterError, FullModel, LlmAdapter};
use crate::symbio_core::authz::PRINCIPAL_AUTONOMOUS;
use crate::symbio_core::Event;

/// 主体模式（[plan/01 §4](../../../../docs/plan/01-核心架构.md)：机制，**3 个封顶**）。
///
/// 模式选择的判据是函数不是清单：可写成确定性规则 → [`Pattern::Decider`]；
/// 只需重述已有事实 → [`Pattern::Translator`]；否则 → [`Pattern::Reasoner`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// 规则 / 代码：确定，毫秒级（意图判定、就绪判定、权限、熔断、仲裁）。
    Decider,
    /// LLM：非确定，秒–分钟（规划、生成、语义判定、工具调用）。
    Reasoner,
    /// 事件 → 另一种表示：确定，百毫秒。**不创造信息**（输出可追溯到输入事件）。
    Translator,
}

/// 作用域（[plan/01 §8](../../../../docs/plan/01-核心架构.md)）。
///
/// `root` 是**一等输入**，不是"没有子智能体"的特例（03 §2 推论：
/// 不出现 `if (children.is_empty())` 分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Root,
    /// 子智能体：挂在其父主体的作用域下。
    Child(String),
}

/// 主体规格（[plan/01 §4](../../../../docs/plan/01-核心架构.md) 的 `ActorSpec`，五字段）。
#[derive(Debug, Clone)]
pub struct ActorSpec {
    /// 身份（数据，无限增长——如 `"agent:main"`）。
    pub principal: String,
    /// 三种模式之一（机制，封顶）。
    pub pattern: Pattern,
    /// 能力名（数据）。S3 之前不做枚举闭集与校验（归 ⑥ `governance`）。
    pub capabilities: Vec<String>,
    /// I3 时延预算（毫秒）。
    pub budget_ms: u64,
    /// 作用域（递归）。
    pub scope: Scope,
}

impl ActorSpec {
    /// S01 的平凡值主体（[roadmap/S01 §4](../../../../docs/plan/roadmap/S01-最小闭环.md)）：
    /// 全规则驱动、预算放宽、单主体——平凡值下系统完整运行。
    pub fn trivial(principal: impl Into<String>) -> Self {
        ActorSpec {
            principal: principal.into(),
            pattern: Pattern::Decider,
            capabilities: Vec::new(),
            budget_ms: 60_000,
            scope: Scope::Root,
        }
    }
}

/// 生成档主体（`pattern = Reasoner` 的 S2 形态，[plan/01 §4](../../../../docs/plan/01-核心架构.md)）。
///
/// ## 闸门在签名上
///
/// [`Reasoner::reply`] 要求 `&FullModel` 令牌——**反射 / 快速档拿不到这个令牌**，
/// 所以「反射层调模型」不是被检测到，而是**编译不过**（J3，[plan/01 §10](../../../../docs/plan/01-核心架构.md)
/// 第 4 条语义）。令牌由 ⑤ `adapters` 的 [`crate::symbio_core::adapters::TokenIssuer`]
/// 按档位签发，装配时注入；`Reasoner` 自身不持有任何适配器句柄。
pub struct Reasoner;

impl Reasoner {
    /// 生成答复：从事件切片取最后一条用户消息作输入，经端口生成。
    ///
    /// 失败形态是 [`AdapterError`]——**调用方必须产出 `chat.assistant.fallback` 事件**
    /// （I3 到点必答：禁止静默超时，[plan/01 §10](../../../../docs/plan/01-核心架构.md) 第 2 条）。
    pub async fn reply(
        &self,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        events: &[Event],
    ) -> Result<String, AdapterError> {
        Ok(self.reply_timed(llm, tok, events).await?.0)
    }

    /// 埋点版 reply（SLO 校准，[plan/04 §1](../../../../docs/plan/04-工程落地.md)）：
    /// 除文本外返回 adapter 边界实测耗时（毫秒）。调用方把它写到 final 事件的
    /// `cost_ms`——成本台账与熔断判据（② CircuitBreaker）从此读**实测值**
    /// 而不是声明值（G3：成本模型有了实测数字）。
    pub async fn reply_timed(
        &self,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        events: &[Event],
    ) -> Result<(String, u64), AdapterError> {
        self.reply_streaming(
            llm,
            tok,
            events,
            std::sync::Arc::new(crate::symbio_core::adapters::SilentDeltas),
        )
        .await
    }

    /// 流式版：生成增量逐片经 `sink` 送出（v2 执行路径的 UI 帧源）；
    /// 返回值与 [`Self::reply_timed`] 同形——流式只是帧的形态，收束语义不变。
    pub async fn reply_streaming(
        &self,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        events: &[Event],
        sink: std::sync::Arc<dyn crate::symbio_core::adapters::DeltaSink>,
    ) -> Result<(String, u64), AdapterError> {
        let messages = Self::render_messages(events);
        llm.generate_streaming(tok, &messages, sink).await
    }

    /// prompt 的**唯一渲染点**（[ADR-048a](../../../../docs/decisions/session.md)）：
    /// 事件切片 → 转写投影 → **消息数组**。
    ///
    /// prompt 从**转写投影**出（多轮带历史）——历史来自同一份事实源，不另存副本
    /// （ADR-044 同族纪律）。**结构化是唯一出口**：拍成一段文本会把
    /// `role: tool` 降级成散文里的纯文本，模型就看不到「这是工具结果」、也拿不到
    /// `tool_call_id`——那是已判定的退步，不是排版偏好。
    ///
    /// ## 为什么运行器仍要就地追加工具交换
    ///
    /// 投影读的是**已落格**的事实（[plan/10 批 3](../../../../docs/plan/10-工具轮v2化实施方案.md)
    /// 起，跨轮的工具结果也在其中）；而**本轮**刚发生的工具交换，快照取在它落格
    /// **之前**（`run_with_tools` 先取 `base_messages`、再逐轮追加），投影里当然没有。
    /// 所以运行器把这一轮的交换就地累积成 `exchange` 追加到基线上——**一条基线 +
    /// 就地累积**，而不是两条渲染路径。续写轮同理：恢复产生的交换也先落格再追加。
    pub fn render_messages(events: &[Event]) -> Vec<crate::symbio_core::PromptMessage> {
        crate::symbio_core::transcript()
            .apply(events, i64::MAX, crate::symbio_core::Budget::generous())
            .value
            .to_messages()
    }
}

/// prompt 的**结构化渲染入口**：事件切片 → 消息数组（[`Reasoner::render_messages`] 的一跳封装）。
///
/// 生产侧（下沉后的 `plugins/session/turn_runner`）只准调本函数，**不得在定义域之外提及
/// 主体类型**——`scripts/no-direct-call-audit.mjs` 的 NDC-001：提及即可持有、持有即可直连
/// （协作只走事件是 I1 的直接推论）。渲染本身是**纯投影**（不碰主体状态），缝只在
/// 「什么时候渲染、往基线上追加什么」由写方决定、「怎么把事件变成消息」由本域决定——
/// 与 [`recalled_event`] 同一形态（生产侧驱动运行器而不逐个持有主体）。
pub fn render_messages(events: &[Event]) -> Vec<crate::symbio_core::PromptMessage> {
    Reasoner::render_messages(events)
}

/// 检索 Translator（`pattern = Translator` 的 S5 形态，[plan/01 §9.2](../../../../docs/plan/01-核心架构.md)）。
///
/// **Translator 不创造信息**：召回内容原样来自 `RecallView`（③ 的投影产出），
/// 本体的职责只是把「读视图」落成「一条事实」——`memory.recalled` 事件
/// （`memory × asserted` 格子，带溯源指向触发它的事件）。检索是
/// 「读视图 → 产出事实」，这正是 Actor 定义对 Translator 的要求。
pub struct RecallTranslator;

impl RecallTranslator {
    /// 把召回结果固化为一条 `memory.recalled` 事件。
    ///
    /// `trigger_seq`：触发本次检索的事件 seq（溯源锚——I2：断言类必带溯源）。
    pub fn recalled_event(
        &self,
        view: &crate::symbio_core::view::RecallView,
        trigger_seq: u64,
        // 谁在断言「我召回了这些」——会话主体（`memory.recalled` 的 `actor`）。
        actor: &str,
    ) -> Event {
        let top = view
            .entries
            .first()
            .map(|e| e.content.as_str())
            .unwrap_or("");
        Event::pending(
            format!("recalled-{trigger_seq}"),
            crate::symbio_core::event::EVENT_MEMORY_RECALLED,
            crate::symbio_core::event::Entity::Memory,
            crate::symbio_core::event::Verb::Asserted,
            0,
            actor,
        )
        .with_produced_by(trigger_seq)
        .with_payload(serde_json::json!({
            "found": view.entries.len(),
            "top": top,
        }))
    }
}

/// `memory.recalled` 的**运行时入口**：读视图 → 一格事实（`RecallTranslator` 的一跳封装）。
///
/// 生产侧只准调本函数，**不得在定义域之外提及主体类型**——那是
/// `scripts/no-direct-call-audit.mjs` 的 NDC-001：提及即可持有，持有即可绕过事实源
/// 直连（协作只走事件是 I1 的直接推论）。分工因此清楚：「什么时候把检索事实落格」
/// 是写方（`plugins/session` 收束时）的决定，「怎么把视图变成一格事实」是本域的
/// 决定，两者的缝就是本函数——与生产侧驱动 [`TurnRunner`] 而不逐个持有主体同一形态。
pub fn recalled_event(
    view: &crate::symbio_core::view::RecallView,
    trigger_seq: u64,
    actor: &str,
) -> Event {
    RecallTranslator.recalled_event(view, trigger_seq, actor)
}

/// 承诺登记者（S6 第 15 步，[roadmap/S08 §3](../../../../docs/plan/roadmap/S08-多主体与对等承诺.md)）。
///
/// 立约 = `commitment` 实体的一格事件；本体的职责只是把「谁向谁承诺了什么 /
/// 是否履行」落成**普通事件**（经 Store，无直连——I1 的直接推论）。
/// 违约不是异常通道：`broken` 与 `released` 是同一格（`commitment × closed`）
/// 的两个名字，违约必须带 `why`（可观测，S08 §5）。
pub struct CommitmentKeeper;

impl CommitmentKeeper {
    /// 立约：`from` 向 `to` 承诺 `promise`。`source_seq` 是触发本次立约的事件
    /// （溯源锚；无触发场景传 0 并由调用方保证可解释）。
    pub fn offer(&self, id: &str, from: &str, to: &str, promise: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("c-offer-{id}"),
            crate::symbio_core::event::EVENT_COMMITMENT_OFFERED,
            crate::symbio_core::event::Entity::Commitment,
            crate::symbio_core::event::Verb::Opened,
            0,
            from,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "id": id, "from": from, "to": to, "promise": promise }))
    }

    /// 守约收束。
    pub fn release(&self, id: &str, from: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("c-close-{id}"),
            crate::symbio_core::event::EVENT_COMMITMENT_RELEASED,
            crate::symbio_core::event::Entity::Commitment,
            crate::symbio_core::event::Verb::Closed,
            0,
            from,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "id": id }))
    }

    /// 违约收束（**必须带 why**——违约可被观测是 T5 的全部前提）。
    pub fn breach(&self, id: &str, from: &str, why: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("c-close-{id}"),
            crate::symbio_core::event::EVENT_COMMITMENT_BROKEN,
            crate::symbio_core::event::Entity::Commitment,
            crate::symbio_core::event::Verb::Closed,
            0,
            from,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "id": id, "why": why }))
    }

    /// 对等宣告：把承诺状态告知协作方（`commitment.asserted`，
    /// `commitment × asserted` 格——声明仍是一条普通事件，带溯源）。
    pub fn declare(&self, id: &str, from: &str, statement: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("c-assert-{id}"),
            crate::symbio_core::event::EVENT_COMMITMENT_ASSERTED,
            crate::symbio_core::event::Entity::Commitment,
            crate::symbio_core::event::Verb::Asserted,
            0,
            from,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "id": id, "from": from, "statement": statement }))
    }
}

/// `commitment.*` 一族的**运行时入口**（[plan/04 §3.1 批⑧](../../../../docs/plan/04-工程落地.md)，
/// NDC-001：定义域外不得提及主体类型名——提及即可持有，持有即可绕过事实源直连）。
///
/// 返回**待入格的事件序列**：立约（`opened`）→ 了结（`released` / `broken`）→
/// 必要时宣告（`asserted`）。写方（`plugins/session` 的收束转写）只管按序 append，
/// 「什么时候落格」是它的决定，「怎么把一次立约变成格子」是本域的决定——与
/// [`recalled_event`] 同一形态。
///
/// 违约才宣告：守约无需告知（履行的东西就摆在那里），违约却必须让承诺对象知道
/// （S08 §5：违约可被观测是 T5 的全部前提）。宣告仍是同一份事实源里的**一格**，
/// 不是新通道（S08 §2「通信 = 没有直连」）。
///
/// 生产侧只准调本函数。
pub fn commitment_events(
    id: &str,
    from: &str,
    to: &str,
    promise: &str,
    ok: bool,
    why: &str,
    source_seq: u64,
) -> Vec<Event> {
    let keeper = CommitmentKeeper;
    let mut out = vec![keeper.offer(id, from, to, promise, source_seq)];
    if ok {
        out.push(keeper.release(id, from, source_seq));
    } else {
        out.push(keeper.breach(id, from, why, source_seq));
        out.push(keeper.declare(id, from, &format!("已告知 {to}：违约——{why}"), source_seq));
    }
    out
}

/// 抢占判定结论（反射档三选一 + 超时默认，[plan/04 §2.1](../../../../docs/plan/04-工程落地.md)）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preemption {
    /// 当前无在跑任务——插话放行，直接开始新 turn。
    Proceed,
    /// 挂起当前任务：`as_of_seq` 是挂起锚 T（恢复 = 以 T 重放投影，零状态迁移）。
    Suspend { task_id: String, as_of_seq: u64 },
    /// 排队：任务不可中断 / final 已发出（已发出的发言不可撤回）。
    Queue,
    /// 判定超时：**默认「继续」不挂起**——挂起会留下需要恢复的状态，继续不会；
    /// 两个选择都可能错，错的代价不对称，默认选代价小的（04 §2.1）。
    TimeoutDefaultContinue,
}

/// 抢占判定者（S8 第 19 步，[roadmap/S07 §3](../../../../docs/plan/roadmap/S07-插话与实时打断.md)）。
///
/// `pattern = decider`、`capabilities = [JudgeIntent]`、`budget_ms = 80`——
/// 系统第一次需要在几十毫秒内对外部信号做决策。**判定者只产控制事件**
/// （`task.controlled`），无 `reply.*` 写权——它不得直接发言（S07 §5，由
/// grants 表保证，见 governance 测试）。
pub struct PreemptionDecider;

impl PreemptionDecider {
    /// 对当前在跑任务的处置判定。
    ///
    /// `elapsed_ms`：判定者自身耗时（调用方 `Instant` 计时后传入——判定是纯函数，
    /// 计时留在边界上）。预算内（≤ `budget_ms`）才做实质判定，超时走默认分支。
    /// 判定顺序（04 §2.1）：无在跑任务 → 放行；final 已发出 → 排队；否则 → 挂起。
    pub fn decide(&self, events: &[Event], elapsed_ms: u64, budget_ms: u64) -> Preemption {
        if elapsed_ms > budget_ms {
            return Preemption::TimeoutDefaultContinue;
        }
        // 在跑任务 = 最新一个 opened 且未终态、未挂起的任务。
        let mut suspended: Vec<&str> = Vec::new();
        let mut done: Vec<&str> = Vec::new();
        let mut running: Option<(&str, u64)> = None;
        for e in events {
            let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
                continue;
            };
            if e.entity == crate::symbio_core::event::Entity::Task {
                match e.kind.as_str() {
                    crate::symbio_core::event::EVENT_TASK_OPENED => {
                        running = Some((id, e.seq.map(|s| s.value()).unwrap_or(u64::MAX)));
                    }
                    crate::symbio_core::event::EVENT_TASK_ASSERTED => {
                        done.push(id);
                        suspended.retain(|x| *x != id);
                    }
                    crate::symbio_core::event::EVENT_TASK_HELD => suspended.push(id),
                    crate::symbio_core::event::EVENT_TASK_PROGRESS => {
                        suspended.retain(|x| *x != id);
                    }
                    _ => {}
                }
            }
        }
        let Some((task_id, opened_seq)) =
            running.take_if(|(id, _)| !done.contains(id) && !suspended.contains(id))
        else {
            return Preemption::Proceed;
        };
        // final 已发出（任务开启后有过收束发言）→ 排队：已发出的发言不可撤回。
        let final_after_task = events.iter().any(|e| {
            e.kind == crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                && e.seq.map(|s| s.value()).unwrap_or(u64::MAX) > opened_seq
        });
        if final_after_task {
            return Preemption::Queue;
        }
        Preemption::Suspend {
            task_id: task_id.to_string(),
            as_of_seq: opened_seq,
        }
    }

    /// 挂起事件（`task × held`）——挂起就是一条事件，不需要新状态机。
    ///
    /// 两个锚各司其职，别混：
    /// - `source_seq` = **挂起前的 head**（04 §2.2 步 1「记录当前 seq 为 T」）——
    ///   它同时是幂等键的后半段，因此必须**每次挂起都不同**：同一个任务可以被挂起
    ///   多次（挂起 → 恢复 → 再挂起），拿任务开格 seq 当锚会让第二次撞 `Duplicate`
    ///   而**静默丢掉**那次挂起（J3）。
    /// - `as_of_seq` = 任务开格 seq（[`Self::decide`] 给出的 as-of 重放锚，04 §2.2
    ///   步 5）——它是判据的一部分，随事实一起落盘，不留在内存里。
    pub fn held_event(&self, task_id: &str, source_seq: u64, as_of_seq: u64) -> Event {
        Event::pending(
            format!("held-{task_id}-{source_seq}"),
            crate::symbio_core::event::EVENT_TASK_HELD,
            crate::symbio_core::event::Entity::Task,
            crate::symbio_core::event::Verb::Held,
            0,
            "agent:reflex",
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "task_id": task_id, "as_of": as_of_seq }))
    }

    /// 恢复事件（`task × progressed`）——挂起的逆操作，同样是一条事件。
    ///
    /// 与 [`Self::held_event`] 成对：**只挂不收就是永久挂起**（挂起期间任务不进
    /// `readyset`，而 `readyset` 又是模型唯一的调度候选来源 ⇒ 任务从此再也不被
    /// 调度，且没有任何一条消息说得出为什么）。故插话轮一结束就由调用方写本条。
    /// 锚同样取写入时刻的 head：同一任务的第二次挂起要有第二次恢复。
    pub fn resume_event(&self, task_id: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("res-{task_id}-{source_seq}"),
            crate::symbio_core::event::EVENT_TASK_PROGRESS,
            crate::symbio_core::event::Entity::Task,
            crate::symbio_core::event::Verb::Progressed,
            0,
            "agent:reflex",
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "task_id": task_id }))
    }

    /// 控制事件（`control × opened`）——打断处置的产出事实。
    pub fn control_event(&self, reason: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("ctrl-{source_seq}"),
            crate::symbio_core::event::EVENT_CONTROL_OPENED,
            crate::symbio_core::event::Entity::Control,
            crate::symbio_core::event::Verb::Opened,
            0,
            "agent:reflex",
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "reason": reason }))
    }
}

/// 闸门结论（S8 第 20 步，[roadmap/S09 §6](../../../../docs/plan/roadmap/S09-外部执行与熔断.md)）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// 未授权——调用方**不得产生任何事件**（验收 1：拒绝且无事件）。
    Refuse,
    /// 熔断（预算耗尽 / 判定超时）——调用方**必须**写 `task.controlled` 事件
    /// （载荷 `reason`），不允许静默继续（验收 2）。
    Break { reason: &'static str },
    /// 放行执行。
    Allow,
}

/// 熔断闸门（S8 第 20 步，[roadmap/S09](../../../../docs/plan/roadmap/S09-外部执行与熔断.md)）。
///
/// `budget_ms = 80` 的反射档判定：超预算 / 超时**熔断**而不是"先做了再说"。
/// 授权判定留在调用方（读路径持矩阵）——本体的输入是判据数据，不是 ⑥ 的句柄。
pub struct CircuitBreaker;

impl CircuitBreaker {
    /// 外部执行闸门。
    ///
    /// - `authorized`：主体是否持外部执行能力（`ProduceArtifact`，读路径判定）；
    /// - `spent_ms` / `requested_ms` / `budget_ms`：已耗 / 本次申请 / 总预算；
    /// - `elapsed_ms`：闸门自身耗时——超反射档预算也熔断（**有事件**的超时，
    ///   不是静默失效；I3）。
    pub fn gate(
        &self,
        authorized: bool,
        spent_ms: u64,
        requested_ms: u64,
        budget_ms: u64,
        elapsed_ms: u64,
    ) -> GateDecision {
        if elapsed_ms > crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms() {
            return GateDecision::Break {
                reason: "gate-timeout",
            };
        }
        if !authorized {
            return GateDecision::Refuse;
        }
        if spent_ms.saturating_add(requested_ms) > budget_ms {
            return GateDecision::Break {
                reason: "budget-exhausted",
            };
        }
        GateDecision::Allow
    }

    /// 熔断事件（复用 `control/opened` 格，载荷 `reason` 区分打断与熔断）。
    pub fn break_event(&self, reason: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("cb-{source_seq}"),
            crate::symbio_core::event::EVENT_CONTROL_OPENED,
            crate::symbio_core::event::Entity::Control,
            crate::symbio_core::event::Verb::Opened,
            0,
            "agent:reflex",
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "reason": reason }))
    }
}

/// 自主发起者（S9 第 21 步，[roadmap/S12](../../../../docs/plan/roadmap/S12-自主层与长期目标.md)）。
///
/// 形状 = `pattern: decider`（规则驱动，零 LLM）、能力只 `define.work`、
/// `budget_ms` = 自主档 [`LatencyTier::Autonomic`]（四层时延的第四个取值——自主层不是
/// 新架构层，只是同一个参数的第四个取值）。
///
/// ## 两处「单点定义」，本类型各引用一次
///
/// - **主体名**：[`PRINCIPAL_AUTONOMOUS`]——表里那一行与这里四个事件的 `actor` 是同一个
///   常量。名字分两处写，授权表那行就成了一条谁也管不到的死记录（见该常量的文档）。
/// - **预算**：[`LatencyTier::Autonomic`]——`86400000` 不在本文件出现第二次。
///
/// 能力那一半**不在这里**：`capabilities = [define.work]` 的权威是授权表
/// （`authz::ROWS` 里 [`PRINCIPAL_AUTONOMOUS`] 那一行），本类型只负责按那个主体名写事件。
/// 两处各写一份能力清单必然漂移成「表授予 A、闸门判 B」——两边都是合法能力名。
pub struct AutonomousInitiator;

impl AutonomousInitiator {
    /// 定时触发：**触发器产出事件，不是旁路**——自主行为同样走 I1 单通道。
    pub fn trigger(&self, source_seq: u64) -> Event {
        Event::pending(
            format!("sys-{source_seq}"),
            crate::symbio_core::event::EVENT_SYSTEM_TRIGGERED,
            crate::symbio_core::event::Entity::System,
            crate::symbio_core::event::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "kind": "scheduled" }))
    }

    /// 表达一条「欲」（E1：欲是数据，必带溯源——否则过不了 IntentGate）。
    pub fn express_intent(&self, goal: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("want-{source_seq}"),
            crate::symbio_core::event::EVENT_CONATION_EXPRESSED,
            crate::symbio_core::event::Entity::Conation,
            crate::symbio_core::event::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "goal": goal }))
    }

    /// 把闸门批准的意图落成长目标任务（自主层 `budget_ms` 的完整取值）。
    pub fn open_long_goal(&self, task_id: &str, goal: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("o-{task_id}"),
            crate::symbio_core::event::EVENT_TASK_OPENED,
            crate::symbio_core::event::Entity::Task,
            crate::symbio_core::event::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "task_id": task_id,
            "depends_on": [],
            "goal": goal,
            "budget_ms": crate::symbio_core::adapters::LatencyTier::Autonomic.budget_ms(),
        }))
    }

    /// 健康自检：触发时刻测得的状态入格（`system × progressed`）。
    ///
    /// [`Self::trigger`] 说"触发了"，本条说"触发时测得什么"——`system` 实体的两个
    /// 格子一个开一个进（S12 §2 / [plan/01 §6](../../../../docs/plan/01-核心架构.md)
    /// 的 `system` 行）。缺了它，健康自检永远没有事实，而"名字是数据、单点定义"
    /// （ADR-043）又不允许插件自己拼 `"system.health"` 字面量——写入口只能在这里。
    ///
    /// `source_seq` 取触发事实的 seq：自检是**这一次触发**的观测附录，不是独立事件。
    pub fn health_event(&self, source_seq: u64, idle_ms: i64) -> Event {
        Event::pending(
            format!("health-{source_seq}"),
            crate::symbio_core::event::EVENT_SYSTEM_HEALTH,
            crate::symbio_core::event::Entity::System,
            crate::symbio_core::event::Verb::Progressed,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "idle_ms": idle_ms }))
    }
}

/// 「欲」的治理策略（E3：平凡值 `enabled = false`——关掉后系统退化为纯响应式
/// 且仍完整运行；两层开关独立，这是生产环境最需要的开关）。
#[derive(Debug, Clone)]
pub struct ConationPolicy {
    /// 是否允许「欲」升格为任务。
    pub enabled: bool,
    /// 目标长度上界（过宽的目标 = 不可评估的意图，拒绝）。
    pub max_goal_len: usize,
}

impl Default for ConationPolicy {
    fn default() -> Self {
        ConationPolicy {
            enabled: false,
            max_goal_len: 200,
        }
    }
}

/// 候选意图：从 `conation.expressed` 事件**唯一**构造路径读出，
/// 造出来一定**未被批准**——「欲」不得直接变成「行」（E2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConationCandidate {
    /// 源事件 seq（溯源锚）。
    pub seq: u64,
    pub goal: String,
    /// 私有：只有闸门能把它置真。
    approved: bool,
}

impl ConationCandidate {
    /// 唯一构造路径：从欲事件读出候选。**无溯源的欲构造不出候选**（I2 强化）。
    pub fn from_event(e: &Event) -> Option<Self> {
        if e.kind != crate::symbio_core::event::EVENT_CONATION_EXPRESSED {
            return None;
        }
        // 来路不明的意图混不进来（I2 强化：无溯源即 None）。
        e.produced_by?;
        let goal = e
            .payload
            .get("goal")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Some(ConationCandidate {
            seq: e.seq.map(|s| s.value()).unwrap_or(u64::MAX),
            goal,
            approved: false,
        })
    }

    pub fn is_approved(&self) -> bool {
        self.approved
    }
}

/// 闸门能力令牌（ZST，私有构造 → 不可伪造；02 §2.3 E2 的可编译强制：
/// 不持令牌则 `approve` 调用**编译失败**，`size_of == 0` 零运行时开销）。
#[derive(Debug, Clone, Copy, Default)]
pub struct GateWarrant {
    _private: (),
}

/// 闸门评估结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentDecision {
    Approved,
    Rejected(&'static str),
}

/// 经闸门批准后的可执行任务（只有这一条路能造出来）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedIntent {
    pub from_seq: u64,
    pub goal: String,
}

/// 意图闸门：**「欲」与「行」之间唯一的一道门**（02 §2.3 E2）。
pub struct IntentGate;

impl IntentGate {
    /// 评估：关停开关优先；目标过宽拒绝。真实系统里这里接价值偏好 / 预算 / 授权。
    pub fn evaluate(candidate: &ConationCandidate, policy: &ConationPolicy) -> IntentDecision {
        if !policy.enabled {
            return IntentDecision::Rejected("conation disabled");
        }
        if candidate.goal.len() > policy.max_goal_len {
            return IntentDecision::Rejected("goal too broad");
        }
        IntentDecision::Approved
    }

    /// 升格：**唯一**能把 [`ConationCandidate`] 变成 [`ApprovedIntent`] 的函数。
    /// 要求 (a) 评估通过 (b) 持 [`GateWarrant`]（不持令牌 = 编译失败）。
    pub fn approve(
        candidate: &mut ConationCandidate,
        policy: &ConationPolicy,
        _warrant: &GateWarrant,
    ) -> Result<ApprovedIntent, &'static str> {
        match Self::evaluate(candidate, policy) {
            IntentDecision::Approved => {
                candidate.approved = true;
                Ok(ApprovedIntent {
                    from_seq: candidate.seq,
                    goal: candidate.goal.clone(),
                })
            }
            IntentDecision::Rejected(r) => Err(r),
        }
    }

    /// 发牌入口——**故意做成唯一一道**：真要多一道门，就得再写一个发牌函数，
    /// 而那个函数是可见的、可审计的（不是靠约定）。
    pub fn issue_warrant() -> GateWarrant {
        GateWarrant { _private: () }
    }
}

/// 技能编译者（S9 第 22 步，[roadmap/S11](../../../../docs/plan/roadmap/S11-技能编译与自我改进.md)）。
///
/// 编译 = 把成功执行轨迹固化为一条 `memory.encoded{tag:"skill"}` 事件——
/// **技能是事实，不是特殊类型**。编译产出的技能事件**必带溯源**
/// （`produced_by` 指向源轨迹；I2 断言，技能溯源 100%）。
pub struct SkillCompiler;

impl SkillCompiler {
    /// 编译一条技能。`source_seq`：源轨迹（成功任务的终态事件）seq。
    ///
    /// 信封里的 `actor` / `turn` 是**占位**（本机主智能体 / `0`）：核心契约只管
    /// 「这条事件说什么」，记在谁头上、发生在哪一轮由部署的写方落格前归位
    /// （`plugins/session/v2_skills.rs::compile`——`memory × *` 的 actor 是属主，
    /// 召回投影按它过滤，占位不归位技能就进不了召回视图）。
    pub fn compile(&self, skill_id: &str, trigger: &str, response: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("skill-{skill_id}-{source_seq}"),
            crate::symbio_core::event::EVENT_MEMORY_ENCODED,
            crate::symbio_core::event::Entity::Memory,
            crate::symbio_core::event::Verb::Opened,
            0,
            "agent:main",
        )
        .with_produced_by(source_seq) // 溯源 100%：指向源轨迹
        .with_payload(serde_json::json!({
            "content": response,
            "tag": "skill",
            "skill_id": skill_id,
            "trigger": trigger,
        }))
    }
}

/// 路由结论：命中技能走快路（`budget_ms = 80`），否则**必须回退**完整推理
/// （反自动化回退，Beilock & Carr 2001——全自动化在压力下以异常方式失效）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillRoute {
    /// 技能命中：反射档快路。
    SkillFastPath { budget_ms: u64 },
    /// 回退完整推理：快速档。
    ReasonerFallback { budget_ms: u64 },
}

/// 技能路由者：按校准置信度决定走技能还是回退（校准值低于阈值 ⇒ 不得走技能路径）。
pub struct SkillRouter;

impl SkillRouter {
    pub fn route(&self, confidence: f64, threshold: f64) -> SkillRoute {
        if confidence >= threshold {
            SkillRoute::SkillFastPath {
                budget_ms: crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms(),
            }
        } else {
            SkillRoute::ReasonerFallback {
                budget_ms: crate::symbio_core::adapters::LatencyTier::Fast.budget_ms(),
            }
        }
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
