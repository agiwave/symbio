//! 一次性阶段的**事实**——`turn.stage`（D-107 判据 2 的落地）。
//!
//! ## 它解决什么
//!
//! `policy.classify` / `policy.recall` / `policy.compress` 的判据在一轮内恒真：
//! `turn.request_round` 只在收轮后 +1，`auto_compress` 是开关不是水位。于是
//! 「这一步做完没有」若没有来源，外环 `drive` 的第二次迭代就会再执行一遍
//! （答话重复、召回重复、压缩反复调用 no-op）——D-107 §3 的整条推理。
//!
//! 本模块把「已做」落成事实：阶段执行点产出一份 [`StageFact`] 草稿，持写者令牌的
//! 一方（[`super::turn_exec`] 的装配段 / 运行器的 [`super::turn_runner::prepare_turn`]）
//! 在用户格之后把它落成 `turn.stage`。下一问问表时 `done` 从它来——`turn_head`
//! 视图按幕锚折叠这些格，不是从第二份内存簿记（DE-014 §2 禁第二份状态）。
//!
//! ## 判决事实为什么也在这里
//!
//! `classify` 的产出是一枚 [`Verdict`]。此前它只活在 `chat_loop` 的局部变量里：
//! 「这一轮为什么直接答 / 为什么派活」在事实面上无从查证。同一格 `turn.stage`
//! 多带 `reason` + `source`（`rule` / `model` / `fallback` / `absent`）就够——
//! 判决与「已做」描述的是同一件事（这一段执行过），载荷只是说得更细。
//!
//! ## 同一性 = `(turn, round, stage)` + **附加段**
//!
//! 事件 id 是 `st-{turn}-{round}-{stage}`，必要时再缀一段 `identity`（不进载荷）。
//! 续写幕（审批 / 问答恢复）**复用同一轮号**且 `request_round` 从 0 重新开始，
//! 于是召回 / 压缩会重跑（这是既有行为：恢复后的事实源已经变了，本轮视图应当重取）。
//! 重跑时若还撞幂等键 ⇒ [`land`] 按「已做过」处理，既不重复落格也不失败——
//! 但那一格**载荷里的数就没了**（压缩那格带着 `model_calls`：它是一次模型往返在
//! 事实层唯一的数）。于是「会在续写幕重跑」的那几步把 `identity` 缀上本幕的锚
//! （`drive_seq`），各幕各一格，撞键不再发生。
//!
//! ## 生产侧按幕锚读事实，而不是按轮扫全史
//!
//! 按轮扫全史会把续写幕的召回 / 压缩**误判为已做**（首幕落过 `round = 0` 的格），
//! 从而改变恢复语义。所以折叠住在本模块之外：`turn_head` 视图只看本幕锚点（新轮
//! = 用户格 seq；恢复幕 = 恢复阶段事实的 seq）之后落下的事实。本模块只负责把
//! 草稿按同一性落成事实，以及提供测试用的**按轮**折叠 [`done`]。
//!
//! `TurnState` 不再维护 `stages_done` 缓存：缓存与事实是第二份状态，
//! 而「这一步做过没有」在日志里已经有了唯一答案。

use crate::symbio_core::{
    AppendError, Entity, Event, Store, Verb, Verdict, EVENT_TURN_STAGE, REASON_ACK, REASON_CLARIFY,
    REASON_EMPTY, REASON_FROM_CONTEXT, REASON_GREETING, REASON_NEEDS_WORK, REASON_REFUSE,
    REASON_THANKS, REASON_UNCLASSIFIED,
};

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
/// 阶段事实比派发指令集宽：`resume`（这一轮是审批 / 问答恢复）与 `round`
/// （第几次请求）是引擎记账，不派发任何执行体。两个闭集因此分开（b 起
/// 派发侧的行由 `loop_rows` 的 `Decl.name` 指认，本枚举只管事实里记了什么）。
///
/// `policy.report` / `policy.reflex` 不在这里：汇报的到期由 `report_due` 视图
/// 算，快路由 `recall` 的 `result` 判——两者都没有「这一步做过没有」要记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageName {
    Decide,
    Compress,
    Recall,
    /// 这一轮是恢复幕（`result` = `retry` / `approve` / `reject` / `supply` /
    /// `answer`；`retry_subtree` 按新轮处理，不落本格）。
    Resume,
    /// 第几次请求——`round` 的**事实形态**。引擎在每次请求前落。
    Round,
}

impl StageName {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Decide => "decide",
            Self::Compress => "compress",
            Self::Recall => "recall",
            Self::Resume => "resume",
            Self::Round => "round",
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
    /// 这一步的**结论**：`recall` 的 `hit` / `miss`、`resume` 的恢复种类、
    /// `round` 无结论。`None` = 这一步没有可记的结论（载荷不带 `result`）。
    pub(crate) result: Option<String>,
    /// 同一性里的**附加段**（不进载荷）：`resume` 用它区分同一轮同类的两次恢复
    /// （两次 `approve` 是同一种动作，却是两次不同的恢复）。`None` = 同一性只由
    /// `(turn, round, stage[, result])` 决定。
    pub(crate) identity: Option<String>,
    /// 这一步打了几次**模型**（D-122 决策 4 / 5）。
    ///
    /// 它是那次往返在事实层唯一的**数**：`source` 记的是判决的**来源**，读不出
    /// 「有没有打过模型」（模型调了而应答不可解析 ⇒ 判决落 `Escalate{unclassified}`
    /// ⇒ `source=fallback`）。判据侧拿它对账「观测 ⇔ 三格」。
    ///
    /// 只有 `decide` 与 `compress` 这两步带它（其余阶段恒 0，载荷里不写）。
    pub(crate) model_calls: u64,
}

impl StageFact {
    /// 一段没有判决的阶段（`recall` / `compress`）。
    pub(crate) fn ran(stage: StageName, round: usize) -> Self {
        Self {
            stage,
            round,
            verdict: None,
            result: None,
            identity: None,
            model_calls: 0,
        }
    }

    /// 带结论的一段（`recall` 的命中 / `resume` 的种类）。
    pub(crate) fn with_result(mut self, result: &str) -> Self {
        self.result = Some(result.to_string());
        self
    }

    /// 恢复幕：这一轮以哪种恢复动作续写（ 的 `resume_allowed` 来源）。
    ///
    /// `identity` = 本幕的序数：同一轮可以先后恢复两次**同类**动作，只按
    /// `(turn, round, stage, kind)` 记会把第二次静默吸收掉（幂等键撞上就会让
    /// `drive_seq` 指回首幕）。序数只进事件 id，不进载荷。
    pub(crate) fn resume(round: usize, kind: &str, identity: &str) -> Self {
        let mut fact = Self::ran(StageName::Resume, round).with_result(kind);
        fact.identity = Some(identity.to_string());
        fact
    }

    /// 第几次请求（ 的 `round` 来源）。
    ///
    /// `identity` = 本幕的锚（`drive_seq`）：同一轮号的续写幕会从 `round = 0`
    /// 重来，只按 `(turn, round, stage)` 记会撞首幕的幂等键，于是撞格取回首幕
    /// 的 seq，本幕的锚指向了错误的位置。
    pub(crate) fn request_round(round: usize, identity: &str) -> Self {
        let mut fact = Self::ran(StageName::Round, round);
        fact.identity = Some(identity.to_string());
        fact
    }

    /// 轮首判决：理由码与来源一并入格（`None` = 分类器没给出判决，记 `absent`）。
    ///
    /// `model_calls` 是这次判决打了几次模型（0 = 规则短路 / 没有模型服务）。
    pub(crate) fn classify(round: usize, verdict: Option<&Verdict>, model_calls: u64) -> Self {
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
            model_calls,
        }
    }

    /// 压缩格：这一次压缩打了几次模型（D-122 决策 2 / 5）。
    ///
    /// `identity` = 本幕的锚（`drive_seq`）：压缩在**续写幕**会重跑（恢复后事实源
    /// 变了，本轮视图应当重取），而同一性若只按 `(turn, round, stage)` 算，续写幕
    /// 那格会撞首幕的幂等键 ⇒ 载荷（含 `model_calls`）**不落** ⇒ 那一笔模型往返
    /// 在事实层没了数，判据侧对不上账。
    pub(crate) fn compress(round: usize, model_calls: u64, drive_seq: u64) -> Self {
        let mut fact = Self::ran(StageName::Compress, round);
        fact.model_calls = model_calls;
        fact.identity = Some(drive_seq.to_string());
        fact
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
        // `model_calls` 只对**打过模型的那几步**有意义（它是那次往返的数）：
        // 判决与压缩。其余阶段恒 0，载荷里不写这个字段——「恒 0 的字段」是噪声。
        if matches!(self.stage, StageName::Decide | StageName::Compress) {
            payload["model_calls"] = serde_json::Value::from(self.model_calls);
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
/// 的 seq——那一格就是本幕的起点，另造一个锚会让 `turn_head` 把首幕的事实
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

/// 🔴 这一步在事实面上是否**已经做过**（按轮折叠，测试用）。
///
/// ⚠️ 它扫全史：续写幕复用同一轮号，首幕落过的格会被判成"已做"，而续写幕的语义
/// 恰恰是「恢复后事实源变了，本轮视图应重取」（召回 / 压缩重跑）。生产侧读的是
/// `turn_head` 视图的按幕锚折叠——本函数只钉住「事实 → 已做」这个形状。
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

#[cfg(test)]
#[path = "stage_events.test.rs"]
mod tests;
