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
//! ## 生产侧的 `done` 为什么是**本次执行内的缓存折叠**
//!
//! [`done`] 的规范形态读完整事件切片；生产侧直接用它会把续写幕的召回 / 压缩
//! **误判为已做**（首幕落过 `round = 0` 的格），从而改变恢复语义。因此
//! `TurnState` 维护一份「本次执行已做」的折叠（`stages_done`），阶段执行点写入、
//! 问表时读出。PL-014 §3 硬规则 d 明文允许增量投影 + 缓存折叠；禁的是**第二份
//! 完整状态表示**，而这里只是一对 `(round, stage)`。
//!
//! [`done`] 本身仍是唯一的投影定义，由单测钉住——drive 将来要读事实时用的是它。

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
    pub(crate) stage: Capsule,
    /// `TurnState::request_round`（压缩 / 召回的「本轮一次」就是按它计的口径）。
    pub(crate) round: usize,
    /// 仅 `classify` 携带；其余阶段 `None`（载荷不带 `reason` / `source`）。
    pub(crate) verdict: Option<VerdictFact>,
}

impl StageFact {
    /// 一段没有判决的阶段（`recall` / `compress`）。
    pub(crate) fn ran(stage: Capsule, round: usize) -> Self {
        Self {
            stage,
            round,
            verdict: None,
        }
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
            stage: Capsule::Decide,
            round,
            verdict: Some(VerdictFact { reason, source }),
        }
    }

    /// 事件 id：`(turn, round, stage)` 是这一格的**同一性**（见模块文档）。
    pub(crate) fn event_id(&self, turn: u64) -> String {
        format!("st-{turn}-{}-{}", self.round, self.stage.name())
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
    for stage in stages {
        let event = Event::pending(
            stage.event_id(turn),
            EVENT_TURN_STAGE,
            Entity::Turn,
            Verb::Asserted,
            turn,
            actor,
        )
        .with_produced_by(user_seq)
        .with_payload(stage.payload());
        match store.append(event) {
            Ok(_) | Err(AppendError::Duplicate) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 这一步在事实面上是否**已经做过**（投影的规范定义）。
///
/// 生产侧的 `done` 用 `TurnState` 的缓存折叠，而不是每次扫全史——续写幕复用
/// 同一轮号，扫全史会把本幕该重跑的阶段误判为已做（见模块文档）。本函数是
/// 「事实 → 已做」的单点答案，drive 将来改读事实时用它，单测也钉它。
///
/// 因此它在非测试构建里没有调用方：不给死代码闸门开豁免，而是如实标成
/// `cfg(test)`——drive 接上时删掉这一行即可。
#[cfg(test)]
pub(crate) fn done(events: &[Event], turn: u64, stage: Capsule, round: usize) -> bool {
    events.iter().any(|e| {
        e.kind == EVENT_TURN_STAGE
            && e.turn == turn
            && e.verb == Verb::Asserted
            && e.payload.get("stage").and_then(|v| v.as_str()) == Some(stage.name())
            && e.payload.get("round").and_then(|v| v.as_u64()) == Some(round as u64)
    })
}

#[cfg(test)]
#[path = "stage_events.test.rs"]
mod tests;
