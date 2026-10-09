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

use crate::symbio_core::adapters::{AdapterError, FullModel, LatencyTier, LlmAdapter};
use crate::symbio_core::authz::PRINCIPAL_AUTONOMOUS;
use crate::symbio_core::event::{
    Entity, Event, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use crate::symbio_core::store::Store;
use crate::symbio_core::view::visible_to;
use crate::symbio_core::Seq;
use std::borrow::Cow;

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

    /// prompt 的**唯一渲染点**：事件切片 → 转写投影 → 单条 prompt。
    ///
    /// prompt 从**转写投影**出（多轮带历史，单轮裸文本与旧形态等价）——历史来自
    /// 同一份事实源，不另存副本（ADR-044 同族纪律）。
    ///
    /// ## 为什么运行器仍要就地追加工具交换
    ///
    /// 投影读的是**已落格**的事实（[plan/10 批 3](../../../../docs/plan/10-工具轮v2化实施方案.md)
    /// 起，跨轮的工具结果也在其中）；而**本轮**刚发生的工具交换，快照取在它落格
    /// **之前**（`run_with_tools` 先取 `base_prompt`、再逐轮追加），投影里当然没有。
    /// 所以运行器把这一轮的交换就地累积成 `exchange` 追加到基线上——**一条基线 +
    /// 就地累积**，而不是两条渲染路径。续写轮同理：恢复产生的交换也先落格再追加。
    pub fn render_prompt(events: &[Event]) -> String {
        crate::symbio_core::transcript()
            .apply(events, i64::MAX, crate::symbio_core::Budget::generous())
            .value
            .to_prompt()
    }

    /// prompt 的**结构化**渲染点（[ADR-048a](../../../../docs/decisions/session.md)）：
    /// 事件切片 → 转写投影 → **消息数组**。
    ///
    /// ## 为什么与 [`Self::render_prompt`] 并存，而不是替换它
    ///
    /// 两者读**同一份** `entries`，差别只在出口：`render_prompt` 给「只需要一段
    /// 文本」的调用方（诊断、断言、日志、`.workbuddy-ai` 里的回读），`render_messages`
    /// 给**送往模型的路径**。**排版是消费方的事，事实不是**——所以事实只有一份。
    ///
    /// ⚠️ 但**送模型的路径必须走结构化那份**：经 `render_prompt` 的那路会由
    /// `to_prompt()` 把 `role: tool` 拍成散文里的纯文本，模型就看不到「这是工具
    /// 结果」也拿不到 `tool_call_id`。这是已判定的退步，不是排版偏好。
    pub fn render_messages(events: &[Event]) -> Vec<crate::symbio_core::PromptMessage> {
        crate::symbio_core::transcript()
            .apply(events, i64::MAX, crate::symbio_core::Budget::generous())
            .value
            .to_messages()
    }
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
    /// 把 `Reasoner::render_prompt` 的渲染结果整段发出），而 v1 的请求视图层
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
/// C4（[`crate::symbio_core::invariants::unresolved_turns`]）按 **turn 号**配对：
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
    pub fell_back: bool,
    /// 中止（用户主动停止 / 会话销毁）：**既无 final 也无兜底格**——网格只留
    /// 已入格的用户消息，少一格是诚实的缺口（ADR-045 的转写纪律同源）；
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
        // 的 `0` 同一条口径（那里 `0` 一直是「上下文窗口不按轮次截断」）。
        //
        // ⚠️ 原先这里是 `&events[events.len()..]`（**空切片**），于是同一份配置在
        // 两条执行路径上含义相反：v1 读 `context_messages` 作「不截断」、v2 读同一个
        // 数作「历史全丢」。出厂档位翻到 `full` 之后，任何把 `context_messages` 配成 0
        // 的实例，**模型眼前的历史是空的**——静默、无告警、断言全绿（事实照样入格、
        // `session/stats` 照样有数）。
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
    /// 跑一轮：用户消息（声明档位）→ 生成 → 收束事件落格。
    ///
    /// `turn` 是本轮的 turn 号（调用方保证单调递增——它就是 N3 的轮次锚）；
    /// `tier` 是**本轮装配进哪一档**的调度决定，随用户消息入格（ADR-044：
    /// 档位是数据）。令牌 `tok` 是闸门：只有持 `FullModel` 的调用方进得来
    /// （反射/快速档在类型上就到不了这里）。
    pub async fn run<S>(
        &self,
        store: &S,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        turn: u64,
        text: &str,
        tier: LatencyTier,
    ) -> Result<TurnOutcome, crate::symbio_core::store::AppendError>
    where
        S: Store<Event = Event>,
    {
        self.run_streaming(
            store,
            llm,
            tok,
            TurnInput {
                turn,
                text: text.to_string(),
                tier,
                window_turns: None,
                // `run` 是**单请求**入口（彩排 / 无工具轮），没有轮内循环 ⇒ 没有
                // 注入时机。要注入请用 `run_with_tools`。
                inject: None,
                resume: None,
                // 便捷入口跑在平凡值身份上（S08 §4：单主体 = 今天的状态）。
                // 生产不走这里——`v2_exec` 自己构造 `ActorSpec` 再调 [`Self::run_with_tools`]。
                actor: ActorSpec::trivial("agent:main"),
                // 便捷入口没有请求视图层可拼（三段由调用方的 `build_request_view` 给出）。
                prefix: None,
                // 便捷入口**恒**走生成：定稿答话轮要落的两格与「用户格 + 收束格」同形，
                // 但它没有生成这一格，调用方得自己把答话交进来（生产：`v2_exec`）。
                final_reply: None,
            },
            std::sync::Arc::new(crate::symbio_core::adapters::SilentDeltas),
        )
        .await
    }

    /// 流式版：生成增量逐片经 `sink` 送出（v2 执行路径的 UI 帧源）；
    /// 落格语义与 [`Self::run`] 完全同一条路径。
    ///
    /// 无工具轮 = [`Self::run_with_tools`] 的**入参为空的同一路径**（工具清单空、
    /// 无分发方）——「有工具」与「无工具」不是两条执行路径。
    pub async fn run_streaming<S>(
        &self,
        store: &S,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        input: TurnInput,
        sink: std::sync::Arc<dyn crate::symbio_core::adapters::DeltaSink>,
    ) -> Result<TurnOutcome, crate::symbio_core::store::AppendError>
    where
        S: Store<Event = Event>,
    {
        self.run_with_tools(store, llm, tok, input, sink, &[], None)
            .await
    }

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
    /// 基线由 [`Reasoner::render_prompt`] 从转写投影出（历史来自事实源，含**往轮**的
    /// 工具结果——[plan/10 批 3](../../../../docs/plan/10-工具轮v2化实施方案.md) 起）；
    /// 而**本轮**刚发生的交换，投影里当然还没有（快照取在它落格之前），故由本函数
    /// **就地累积**追加——模型必须看到自己请求过什么、拿到了什么，否则会重复调用
    /// 同一个工具。
    ///
    /// ## 中止 / 等待用户
    ///
    /// 两者都**不落收束格**（网格少一格是诚实缺口，ADR-045 同源），差别在原因：
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
        sink: std::sync::Arc<dyn crate::symbio_core::adapters::DeltaSink>,
        tools: &[crate::symbio_core::CapabilityMeta],
        dispatch: Option<&dyn crate::symbio_core::DispatchPort>,
    ) -> Result<TurnOutcome, crate::symbio_core::store::AppendError>
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
        // 基线：**结构化**消息数组（ADR-048a）。不再是「一整段 prompt 字符串」。
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
        let mut base_messages = Reasoner::render_messages(&snapshot);
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
            let rep = crate::symbio_core::projection::prompt_fidelity::verify(
                &snapshot,
                &base_messages,
                declared,
            );
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
        // **结构化**（ADR-048a）：曾几何时它是 `String`，由 `render_tool_exchange`
        // 拍成「助手请求工具: … / 工具结果(x): …」的散文再拼到基线末尾。那条路让
        // 模型在本轮的工具结果里看到的是**自己说的话**（role=assistant）和
        // **一坨没有角色的文本**，且无 `tool_call_id`。现在它们是三条真消息：
        // assistant(tool_calls) / tool(结果, 带 call id)。
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
            // 基线 + 本轮交换。**拼接消息数组**，不拼字符串——拼字符串正是
            // ADR-048a 判定的退步：它把角色拍平、丢掉 `tool_call_id`。
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
                    // 的缺口（ADR-045 同一纪律）。兜底格是**失败**的形状（`fell_back`
                    // 专有），中止落了它就等于把「用户按了停止」记成「模型答不出」，
                    // 还会抬高兜底率——那是假象。
                    let cost = started.elapsed().as_millis() as u64;
                    return Ok(TurnOutcome {
                        turn,
                        text: String::new(),
                        cost_ms: cost,
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
        _tok: &crate::symbio_core::adapters::RuleOnly,
        turn: u64,
        actor: &ActorSpec,
        utterance: &str,
        text: &str,
        sink: std::sync::Arc<dyn crate::symbio_core::adapters::DeltaSink>,
    ) -> Result<TurnOutcome, crate::symbio_core::store::AppendError>
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
    ) -> Result<TurnOutcome, crate::symbio_core::store::AppendError>
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
/// ## 这里曾几何时返回 `String`，由 `render_tool_exchange` 拍成散文
///
/// ```ignore
/// 助手: <text>
/// 助手请求工具: read {args}
/// 工具结果(read): <result>
/// ```
///
/// 那条路的三个问题，**都出在同一个动作上**（拼字符串）：
///
/// 1. `助手请求工具: ` 那行被当成 **assistant 说的话**——模型于是「说过」一句
///    人类不会说的话；
/// 2. `工具结果(read): ` 那行**没有角色**，混在对话流里，模型分不出它是工具输出
///    还是用户输入；
/// 3. **没有 `tool_call_id`**——而 provider 侧的协议要求 `tool` 消息关联到具体
///    那次调用，否则多工具并发时无法配对。
///
/// 现在是三条真消息：assistant（带 tool_calls 摘要）/ tool（带 `tool_call_id`）。
///
/// ## `tool_call_id` 怎么来（**已定的合成，不是缺失**）
///
/// 网格里的 `artifact.added` 只记了工具名与正文——**那才是事实**，没有调用 id
/// （调用 id 是本次运行的句柄，不是跨轮事实）。所以这里按「工具名 + 本轮序号」
/// 合成一个**稳定 id**（见 [`synthetic_call_id`]），而不是让消息裸奔：
/// 裸奔会被「拒绝未知 tool_call_id」的 provider 整条拒绝，那**比拍平更糟**
/// （拍平至少还能答）。
fn tool_exchange_messages(
    turn: &crate::symbio_core::adapters::LlmTurn,
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
            tool_calls: Some(vec![
                crate::symbio_core::projection::transcript::PromptToolCall {
                    id,
                    name: name.to_string(),
                    arguments: call.arguments.clone(),
                },
            ]),
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
            tool_calls: Some(vec![
                crate::symbio_core::projection::transcript::PromptToolCall {
                    id: id.clone(),
                    name: name.to_string(),
                    arguments: call.arguments.clone(),
                },
            ]),
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
#[path = "mod.test.rs"]
mod tests;
