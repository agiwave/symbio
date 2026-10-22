//! 一次性阶段的**事实**——`turn.stage`（ADR-107 判据 2 的落地）。
//!
//! ## 它解决什么
//!
//! `policy.classify` / `policy.recall` / `policy.compress` 的判据在一轮内恒真：
//! `turn.request_round` 只在收轮后 +1，`auto_compress` 是开关不是水位。于是
//! 「这一步做完没有」若没有来源，外环 `drive` 的第二次迭代就会再执行一遍
//! （答话重复、召回重复、压缩反复调用 no-op）——ADR-107 §3 的整条推理。
//!
//! 本模块把「已做」落成事实：阶段执行点产出一份 [`StageFact`] 草稿，持写者令牌的
//! 一方（运行器的 [`super::turn_runner::prepare_turn`] / `run_reflex`）在用户格
//! 之后把它落成 `turn.stage`。下一问问表时 `done` 从它来——不是从第二份内存簿记，
//! 也不是从 `LoopState` 里多出来的一个字段（PL-014 §2 禁第二份状态）。
//!
//! ## 判决事实为什么也在这里
//!
//! `classify` 的产出是一枚 [`Verdict`]。此前它只活在 `chat_loop` 的局部变量里：
//! 「这一轮为什么直接答 / 为什么派活」在事实面上无从查证。同一格 `turn.stage`
//! 多带 `reason` + `source`（`rule` / `model` / `fallback` / `absent`）就够——
//! 判决与「已做」描述的是同一件事（这一段执行过），载荷只是说得更细。
//!
//! ## 同一性 = `(turn, round, stage)`
//!
//! 事件 id 是 `st-{turn}-{round}-{stage}`。续写幕（审批 / 问答恢复）**复用同一轮号**
//! 且 `request_round` 从 0 重新开始，于是召回 / 压缩会重跑（这是既有行为：恢复后
//! 的事实源已经变了，本轮视图应当重取）。重跑时撞幂等键 ⇒ [`land`] 按「已做过」
//! 处理，既不重复落格也不失败。
//!
//! ## 生产侧读事实，但**按本次执行的锚**读
//!
//! 按轮扫全史会把续写幕的召回 / 压缩**误判为已做**（首幕落过 `round = 0` 的格），
//! 从而改变恢复语义。因此生产侧用 [`done_since`]：只看本幕锚点（新轮 = 用户格
//! seq；恢复幕 = 恢复阶段事实的 seq）之后落下的事实。
//!
//! `TurnState` 不再维护 `stages_done` 缓存（T-225）：缓存与事实是第二份状态，
//! 而「这一步做过没有」在日志里已经有了唯一答案。

use crate::symbio_core::{
    AppendError, Entity, Event, Store, Verb, Verdict, EVENT_TURN_STAGE, REASON_ACK, REASON_CLARIFY,
    REASON_EMPTY, REASON_FROM_CONTEXT, REASON_GREETING, REASON_NEEDS_WORK, REASON_REFUSE,
    REASON_THANKS, REASON_UNCLASSIFIED,
};

use super::loop_registry::Capsule;

/// 判决的**来源**：理由码由规则产出、由模型产出、兜底，或根本没有判决。
///
/// ## 为什么是派生的，而不是 `Verdict` 的字段
///
/// 给 `Verdict` 加 `source` 会让每个产出点都要负责填一个它并不关心的量，且把
/// `classify` 的实现细节（规则短路 vs 模型）写进 core 的契约面。理由码词表本身
/// 已经**划分**了两侧：四个规则码 + 四个模型码 + 一个兜底码。映射住在本模块，
/// 由 `every_reason_has_a_source` 对整张词表逐条钉住——词表改了而映射没跟上，
/// 那条用例会红。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerdictSource {
    /// 规则短路产出（`classify` 的规则表命中，零模型往返）。
    Rule,
    /// 模型判决产出。
    Model,
    /// 兜底（分类器不可用 / 响应不可解析 / 无发言）。
    Fallback,
    /// 没有判决（未挂载 `classify` / 路由失败 / 契约不符）。
    Absent,
}

impl VerdictSource {
    /// 来源名——载荷与日志的唯一取值点。
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Model => "model",
            Self::Fallback => "fallback",
            Self::Absent => "absent",
        }
    }
}

/// **一格阶段的名字**——`turn.stage` 载荷 `stage` 的闭集。
///
/// ## 为什么不是直接复用 [`Capsule`]
///
/// `Capsule` 是**环认识的指令集**（一条臂 = 一段会派发的执行体）。阶段事实比它宽：
/// `resume`（这一轮是审批 / 问答恢复）与 `round`（第几次请求）是引擎记账，不派发
/// 任何臂；把它们塞进 `Capsule` 会凭空多出两条"永不派发"的指令，而 `Capsule::ALL`
/// 的对偶检查（每条臂必须有产出行）随即说谎。
///
/// 两个闭集因此分开：`Capsule` 管"环会做什么"，`StageName` 管"事实里记了什么"。
/// 前者与后者之间只有单向转换（[`From<Capsule>`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageName {
    Decide,
    Report,
    Compress,
    Recall,
    Reflex,
    /// 这一轮是恢复幕（`result` = `retry` / `approve` / `reject` / `supply` /
    /// `answer`；`retry_subtree` 按新轮处理，不落本格）。
    Resume,
    /// 第几次请求——`round` 的**事实形态**（T-225）。引擎在每次请求前落。
    Round,
}

impl StageName {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Decide => "decide",
            Self::Report => "report",
            Self::Compress => "compress",
            Self::Recall => "recall",
            Self::Reflex => "reflex",
            Self::Resume => "resume",
            Self::Round => "round",
        }
    }
}

impl From<Capsule> for StageName {
    fn from(c: Capsule) -> Self {
        match c {
            Capsule::Decide => Self::Decide,
            Capsule::Report => Self::Report,
            Capsule::Compress => Self::Compress,
            Capsule::Recall => Self::Recall,
            Capsule::Reflex => Self::Reflex,
        }
    }
}

/// 一次判决的事实内容：理由码 + 它由谁产出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerdictFact {
    /// 理由码（`Verdict::Answered` / `Escalate` 的 `reason`；`None` = 没有判决）。
    pub(crate) reason: Option<String>,
    pub(crate) source: VerdictSource,
}

/// 一格待落 `turn.stage` 的草稿。
///
/// 由阶段执行点（`chat_loop` / `chat_loop::inputs`）产出，落格在持写者令牌的
/// 运行器一侧——与 `ReportDraft` / `DialogFacts` 同一条分割：判定与措辞在编排层，
/// 落格在 `turn_exec`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StageFact {
    pub(crate) stage: StageName,
    /// `TurnState::request_round`（压缩 / 召回的「本轮一次」就是按它计的口径）。
    pub(crate) round: usize,
    /// 仅 `classify` 携带；其余阶段 `None`（载荷不带 `reason` / `source`）。
    pub(crate) verdict: Option<VerdictFact>,
    /// 这一步的**结论**（T-225）：`recall` 的 `hit` / `miss`、`resume` 的恢复种类、
    /// `round` 无结论。`None` = 这一步没有可记的结论（载荷不带 `result`）。
    pub(crate) result: Option<String>,
    /// 同一性里的**附加段**（不进载荷）：`resume` 用它区分同一轮同类的两次恢复
    /// （两次 `approve` 是同一种动作，却是两次不同的恢复）。`None` = 同一性只由
    /// `(turn, round, stage[, result])` 决定。
    pub(crate) identity: Option<String>,
}

impl StageFact {
    /// 一段没有判决的阶段（`recall` / `compress`）。
    pub(crate) fn ran(stage: impl Into<StageName>, round: usize) -> Self {
        Self {
            stage: stage.into(),
            round,
            verdict: None,
            result: None,
            identity: None,
        }
    }

    /// 带结论的一段（`recall` 的命中 / `resume` 的种类）。
    pub(crate) fn with_result(mut self, result: &str) -> Self {
        self.result = Some(result.to_string());
        self
    }

    /// 恢复幕：这一轮以哪种恢复动作续写（T-225 的 `resume_allowed` 来源）。
    ///
    /// `identity` = 本幕的序数：同一轮可以先后恢复两次**同类**动作，只按
    /// `(turn, round, stage, kind)` 记会把第二次静默吸收掉（幂等键撞上就会让
    /// `drive_seq` 指回首幕）。序数只进事件 id，不进载荷。
    pub(crate) fn resume(round: usize, kind: &str, identity: &str) -> Self {
        let mut fact = Self::ran(StageName::Resume, round).with_result(kind);
        fact.identity = Some(identity.to_string());
        fact
    }

    /// 第几次请求（T-225 的 `round` 来源）。
    ///
    /// `identity` = 本幕的锚（`drive_seq`）：同一轮号的续写幕会从 `round = 0`
    /// 重来，只按 `(turn, round, stage)` 记会撞首幕的幂等键，于是本幕的锚
    /// 指回首幕，`done_since` 把首幕的阶段误判进本幕。
    pub(crate) fn request_round(round: usize, identity: &str) -> Self {
        let mut fact = Self::ran(StageName::Round, round);
        fact.identity = Some(identity.to_string());
        fact
    }

    /// 轮首判决：理由码与来源一并入格（`None` = 分类器没给出判决，记 `absent`）。
    pub(crate) fn classify(round: usize, verdict: Option<&Verdict>) -> Self {
        let reason = verdict.and_then(|v| match v {
            Verdict::Answered { reason } | Verdict::Escalate { reason } => Some(reason.clone()),
            // `Report` 由 session 在轮边界产出，不进轮首判决；没有理由码可记。
            Verdict::Report => None,
        });
        let source = reason
            .as_deref()
            .map(source_of)
            .unwrap_or(VerdictSource::Absent);
        Self {
            stage: StageName::Decide,
            round,
            verdict: Some(VerdictFact { reason, source }),
            result: None,
            identity: None,
        }
    }

    /// 事件 id：`(turn, round, stage)` 是这一格的**同一性**（见模块文档）。
    ///
    /// `resume` 把 `result` 与幕序数也纳入同一性：同一轮可以先后恢复两次，两次
    /// 恢复说的是两件事；只按 `(turn, round, stage)` 记会把第二次静默吸收掉。
    pub(crate) fn event_id(&self, turn: u64) -> String {
        let mut id = format!("st-{turn}-{}-{}", self.round, self.stage.name());
        if self.stage == StageName::Resume {
            if let Some(kind) = &self.result {
                id.push('-');
                id.push_str(kind);
            }
        }
        if let Some(identity) = &self.identity {
            id.push('-');
            id.push_str(identity);
        }
        id
    }

    pub(crate) fn payload(&self) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "stage": self.stage.name(),
            "round": self.round,
        });
        if let Some(v) = &self.verdict {
            payload["source"] = serde_json::Value::String(v.source.name().to_string());
            if let Some(reason) = &v.reason {
                payload["reason"] = serde_json::Value::String(reason.clone());
            }
        }
        if let Some(result) = &self.result {
            payload["result"] = serde_json::Value::String(result.clone());
        }
        payload
    }
}

/// 理由码 → 来源。划分依据是 `classify` 的产出路径（规则短路 / 模型），见
/// [`VerdictSource`] 的文档。
pub(crate) fn source_of(reason: &str) -> VerdictSource {
    match reason {
        REASON_GREETING | REASON_THANKS | REASON_ACK | REASON_EMPTY => VerdictSource::Rule,
        REASON_FROM_CONTEXT | REASON_CLARIFY | REASON_REFUSE | REASON_NEEDS_WORK => {
            VerdictSource::Model
        }
        REASON_UNCLASSIFIED => VerdictSource::Fallback,
        // 未登记的理由码按兜底记：来源不明的判决**不是**「规则」也不是「模型」，
        // 把它记成任一侧都会让来源统计说谎。
        _ => VerdictSource::Fallback,
    }
}

/// 把一批阶段草稿落成 `turn.stage`（在用户格之后调用——I2 要求断言类事件带溯源）。
///
/// `Duplicate` 不是错误：同一 `(turn, round, stage)` 已落意味着这一步做过，
/// 续写幕重跑同一轮时会命中（见模块文档）。
pub(crate) fn land<S>(
    store: &S,
    turn: u64,
    user_seq: u64,
    actor: &str,
    stages: &[StageFact],
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    land_with_seqs(store, turn, user_seq, actor, stages).map(|_| ())
}

/// 与 [`land`] 同一个落格动作，但把每一格的 **seq** 交回来。
///
/// 调用方（`chat_loop` 的恢复幕）需要它：`drive_seq` 就是「本幕第一条阶段事实」
/// 的位置，而那个位置只有落格方拿得到。`Duplicate`（同一格重跑）取**已落那一格**
/// 的 seq——那一格就是本幕的起点，另造一个锚会让 `done_since` 把首幕的事实
/// 误算进本幕。
pub(crate) fn land_with_seqs<S>(
    store: &S,
    turn: u64,
    user_seq: u64,
    actor: &str,
    stages: &[StageFact],
) -> Result<Vec<u64>, AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let mut seqs = Vec::with_capacity(stages.len());
    for stage in stages {
        let id = stage.event_id(turn);
        let event = Event::pending(
            id.clone(),
            EVENT_TURN_STAGE,
            Entity::Turn,
            Verb::Asserted,
            turn,
            actor,
        )
        .with_produced_by(user_seq)
        .with_payload(stage.payload());
        match store.append(event) {
            Ok(seq) => seqs.push(seq.value()),
            Err(AppendError::Duplicate) => {
                let existing = store
                    .range(crate::symbio_core::Seq::new(0))
                    .into_iter()
                    .find(|e| e.turn == turn && e.kind == EVENT_TURN_STAGE && e.event_id == id)
                    .and_then(|e| e.seq.map(|s| s.value()))
                    .unwrap_or(0);
                seqs.push(existing);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(seqs)
}

/// 🔴 这一步在事实面上是否**已经做过**（投影的规范定义，**按轮**）。
///
/// ⚠️ 它扫全史：续写幕复用同一轮号，首幕落过的格会被判成"已做"，而续写幕的语义
/// 恰恰是「恢复后事实源变了，本轮视图应重取」（召回 / 压缩重跑）。生产侧因此用
/// [`done_since`]——**只在本次执行锚点之后**找同一格。本函数保留为「事实 → 已做」
/// 的规范折叠，供单测与将来的整轮视图用。
#[cfg(test)]
pub(crate) fn done(events: &[Event], turn: u64, stage: StageName, round: usize) -> bool {
    events.iter().any(|e| {
        e.kind == EVENT_TURN_STAGE
            && e.turn == turn
            && e.verb == Verb::Asserted
            && e.payload.get("stage").and_then(|v| v.as_str()) == Some(stage.name())
            && e.payload.get("round").and_then(|v| v.as_u64()) == Some(round as u64)
    })
}

/// 这一步在**本次执行**里是否已做过：只看 `drive_seq` 之后落下的 `turn.stage`。
///
/// `drive_seq` 是本幕的锚（新轮 = 用户格 seq；恢复幕 = 恢复阶段事实的 seq）。
/// 用它的理由就是上面那条：同一轮号会被续写幕复用，按轮扫全史会把首幕的
/// `recall` / `compress` 误判成"已做"，恢复幕于是不再重取视图。
pub(crate) fn done_since(events: &[Event], drive_seq: u64, stage: StageName, round: usize) -> bool {
    events.iter().any(|e| {
        e.seq.is_some_and(|s| s.value() >= drive_seq)
            && e.kind == EVENT_TURN_STAGE
            && e.verb == Verb::Asserted
            && e.payload.get("stage").and_then(|v| v.as_str()) == Some(stage.name())
            && e.payload.get("round").and_then(|v| v.as_u64()) == Some(round as u64)
    })
}

#[cfg(test)]
#[path = "stage_events.test.rs"]
mod tests;
