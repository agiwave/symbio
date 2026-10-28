//! 一次性阶段的**事实**——`turn.stage`（D-107 判据 2 的落地）。
//!
//! ## 它解决什么
//!
//! `policy.classify` / `policy.recall` / `policy.compress` 的判据在一轮内恒真：
//! `turn.request_round` 只在收轮后 +1，`auto_compress` 是开关不是水位。于是
//! 「这一步做完没有」若没有来源，外环 `drive` 的第二次迭代就会再执行一遍
//! （答话重复、召回重复、压缩反复调用 no-op）——D-107 §3 的整条推理。
//!
//! 本模块把「已做」落成事实。落格者只有一种身份：**产它的那一步自己落**。
//!
//! 环上那几行（`policy.classify` / `policy.compress` / `policy.recall` / `policy.report`）
//! 由执行体在动作完成的那个时刻落：派发点两次动作之间重读日志，只有日志看得见这一行关账。
//!
//! 于是一条禁止适用于全模块：**不寄存草稿**。草稿要等另一段落进日志，而派发点读的
//! 就是日志——环会在同一次调用里再选中同一笔债。
//!
//! ## 判决事实为什么也在这里
//!
//! `classify` 的产出是一枚 [`Verdict`]，而「这一轮为什么直接答 / 为什么派活」只有从
//! 日志才查得出来。同一格 `turn.stage` 多带 `reason` + `source`（`rule` / `model` /
//! `fallback` / `absent`）就够——判决与「已做」描述的是同一件事（这一段执行过），
//! 载荷只是说得更细。
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
//! [`StageFact`] 按同一性落成事实，以及提供测试用的**按轮**折叠 [`done`]。
//!
//! 「这一步做过没有」在日志里有唯一答案，所以本轮状态里不留它的缓存——
//! 缓存与事实是第二份状态（DE-014 §2）。

use symbio_core::{
    AppendError, Entity, Event, Store, Verb, Verdict, REASON_ACK, REASON_CLARIFY, REASON_EMPTY,
    REASON_FROM_CONTEXT, REASON_GREETING, REASON_NEEDS_WORK, REASON_REFUSE, REASON_THANKS,
    REASON_UNCLASSIFIED,
};

use crate::control_decl::EVENT_TURN_STAGE;

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
/// （第几次请求）是引擎记账，不派发任何执行体。两个闭集因此分开（事实侧多出这两格，
/// 派发侧的行由 `loop_rows` 的 `Decl.name` 指认，本枚举只管事实里记了什么）。
///
/// `policy.report` 也在这里：它的到期由 `report_due` 视图算，但「这一档静默试过
/// 没有」得记在日志上（[`StageName::Report`]）。快路（`policy.reflex`）在这里：
/// `reflex` 是它的出口格（见 [`StageName::Reflex`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageName {
    Decide,
    Compress,
    Recall,
    /// 反射档这一轮已作答（`result` = 命中的技能 id）。它是 `policy.reflex`
    /// 的出口，也是 `turn_head.reflex_answered` 的唯一来源。
    Reflex,
    /// 这一次静默区间报过（`result` = `sent` / `silent`）。
    ///
    /// 它是 `policy.report` 的出口。`silent`（措辞没给话）也要落：只落 `sent` 的话，
    /// 一个不产话的措辞口会让同一个静默区间每次迭代都重新到期——环在两格之间空转。
    /// 同一性带 `report_due` 的锚（见 [`StageFact::report`]），下一个静默区间是新的一格。
    ///
    /// 它**不是** `reported_anchors` 的关账格：那个视图只认汇报事实（`chat.assistant
    /// .reported`），把这一格算进去会让本族事实成为自己的依赖边（`reported_anchors`
    /// 的出口 ⇒ 报告行的执行体，而报告行又写这一格 ⇒ `DependencyCycle`）。「同一段
    /// 静默只试一次」因此由 `report_due` 在**开账侧**读这一格判，不在关账侧。
    Report,
    /// 这一轮是恢复幕（`result` = `retry` / `approve` / `reject` / `supply` /
    /// `answer`；`retry_subtree` 按新轮处理，不落本格）。
    Resume,
    /// 第几次请求——`round` 的**事实形态**。引擎在每次请求前落。
    Round,
    /// 本幕的请求输入已备好：接缝行的出口。
    ///
    /// `result` 两档：`ready` = 输入就绪、这一幕还欠一次往返；`closed` = 准备段
    /// 走完时本幕已经收束（定稿答话轮）——那一幕**没有**输入，也没有往返。
    /// 两档都要落：只落 `ready` 的话，收束的那一幕在事实面上与「还没准备」同形。
    ///
    /// `ready` 是 `turn_head.prepared` 的来源，而接缝行（`turn × asserted`）以它为
    /// 出口：`prepared: false` ⇒ 还欠这一段，`prepared: true` ⇒ 轮内各行可以开账。
    Inputs,
}

impl StageName {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Decide => "decide",
            Self::Compress => "compress",
            Self::Recall => "recall",
            Self::Reflex => "reflex",
            Self::Report => "report",
            Self::Resume => "resume",
            Self::Round => "round",
            Self::Inputs => "inputs",
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

    /// 恢复幕：这一轮以哪种恢复动作续写（ `resume_allowed` 来源）。
    ///
    /// `identity` = 本幕的序数：同一轮可以先后恢复两次**同类**动作，只按
    /// `(turn, round, stage, kind)` 记会把第二次静默吸收掉（幂等键撞上就会让
    /// `drive_seq` 指回首幕）。序数只进事件 id，不进载荷。
    pub(crate) fn resume(round: usize, kind: &str, identity: &str) -> Self {
        let mut fact = Self::ran(StageName::Resume, round).with_result(kind);
        fact.identity = Some(identity.to_string());
        fact
    }

    /// 第几次请求（ `round` 来源）。
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

    /// 召回格：本幕的技能命中与否（`result` = `hit` / `miss`）。
    ///
    /// 同一性带上本幕的锚（`drive_seq`）——与 [`Self::compress`] 同一条理由：召回在
    /// 续写幕会重跑（恢复后事实源变了，本轮视图应当重取），只按 `(turn, round, stage)`
    /// 记就撞首幕那一格 ⇒ 本幕的 `result` **不落**。而这一行的收口与 `policy.reflex`
    /// 的 `skill_hit` 都只读**本幕锚之后**的那一格（`turn_stages` / `turn_head` 按幕锚
    /// 折叠）：撞掉的不是载荷，是判据本身。
    pub(crate) fn recall(round: usize, drive_seq: u64, result: &str) -> Self {
        let mut fact = Self::ran(StageName::Recall, round).with_result(result);
        fact.identity = Some(drive_seq.to_string());
        fact
    }

    /// 反射档格：这一轮由技能正文直接作答（`result` = 命中的技能 id）。
    ///
    /// 同一性带上本幕的锚（`drive_seq`）——与 [`Self::recall`] 同一条理由：快路
    /// 只在**新开轮**成立（`policy.reflex` 的 `resumed = false`），续写幕不会重跑；
    /// 但同一轮若因事实源补落重放，撞格取回首幕才是对的语义。
    ///
    /// 这一格是 `policy.reflex` 的**出口**：执行体只落「已答」，收束格由
    /// `close-turn` 统一落（D-163）。
    pub(crate) fn reflex(round: usize, drive_seq: u64, skill_id: &str) -> Self {
        let mut fact = Self::ran(StageName::Reflex, round).with_result(skill_id);
        fact.identity = Some(drive_seq.to_string());
        fact
    }

    /// 汇报尝试格：这一次静默区间报过（`result` = `sent` / `silent`）。
    ///
    /// `identity` = 这段静默的**锚**（`report_due.anchor`，最近一条可作锚的发言 /
    /// 汇报 / 补充格的 seq）。它是「同一段静默只试一次」在事实面上的身份：
    /// `report_due` 读到锚之后已有这一格，就不再判到期。
    ///
    /// `silent`（措辞没给话）也落一格——只落 `sent` 的话，一个不产话的措辞口会让
    /// 同一段静默每次选债都重新到期，环在两格之间空转。
    pub(crate) fn report(round: usize, anchor: u64, result: &str) -> Self {
        let mut fact = Self::ran(StageName::Report, round).with_result(result);
        fact.identity = Some(anchor.to_string());
        fact
    }

    /// 本幕的请求输入已备好：接缝行的出口（同名文档见 [`StageName::Inputs`]）。
    ///
    /// 同一性带上本幕的锚（`drive_seq`）——与 [`Self::recall`] 同一条理由，但这里
    /// 更硬：续写幕复用轮号与请求轮次，`(turn, round, stage)` 三元组会与首幕
    /// **撞成同一格**，而 `land` 把 `Duplicate` 当成功吞掉 ⇒ 续写幕的准备「做完了」
    /// 这件事根本不进日志（e2e t27 抓到的正是这一条：`prepare 落了零格`）。
    pub(crate) fn inputs(round: usize, drive_seq: u64, result: &str) -> Self {
        let mut fact = Self::ran(StageName::Inputs, round).with_result(result);
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
                    .range(symbio_core::Seq::new(0))
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
