//! 一次性步的**事实**——`loop.step`（D-107 判据 2 的落地）。
//!
//! ## 它解决什么
//!
//! `policy.classify` / `policy.recall` / `policy.compress` 的判据在一轮内恒真：
//! `turn.round()` 只在收轮后 +1，`auto_compress` 是开关不是水位。于是
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
//! 日志才查得出来。同一格 `loop.step` 多带 `reason` + `source`（`rule` / `model` /
//! `fallback` / `absent`）就够——判决与「已做」描述的是同一件事（这一段执行过），
//! 载荷只是说得更细。
//!
//! ## 同一性 = `(turn, id, 幕锚)` + **附加段**
//!
//! 事件 id 是 `st-{turn}-{id}-{幕锚或 identity}`。轮号**不进 id**：它是预算的数，
//! 不是键（同一轮可以先后开好几幕，轮号会重复；幕锚不会）。
//! 续写幕（审批 / 问答恢复）复用同一轮号且 `round()` 从 0 重新开始，
//! 于是召回 / 压缩会重跑（这是既有行为：恢复后的事实源已经变了，本轮视图应当重取）。
//! 重跑时若还撞幂等键 ⇒ [`land`] 按「已做过」处理，既不重复落格也不失败——
//! 但那一格**载荷里的数就没了**（压缩那格带着 `model_calls`：它是一次模型往返在
//! 事实层唯一的数）。于是「会在续写幕重跑」的那几步把 `identity` 缀上本幕的锚
//! （`drive_seq`），各幕各一格，撞键不再发生。
//!
//! ## 生产侧按幕锚读事实，而不是按轮扫全史
//!
//! 按轮扫全史会把续写幕的召回 / 压缩**误判为已做**（首幕落过同一轮号的格），
//! 从而改变恢复语义。所以折叠住在本模块之外：`turn_head` 视图只看本幕锚点（新轮
//! = 用户格 seq；恢复幕 / 续幕 = 那一格 `loop.step` 自己的 seq）之后、且载荷
//! `anchor` 等于该锚的事实。本模块只负责把 [`StageFact`] 按同一性落成事实，
//! 以及提供测试用的**按轮**折叠 [`done`]。
//!
//! 「这一步做过没有」在日志里有唯一答案，所以本轮状态里不留它的缓存——
//! 缓存与事实是第二份状态（DE-014 §2）。写法也只有这一种（`D-228`）：端口的往返三格
//! 答的是「在途」，不答「已做」；旧视图 `turn_round` 已随 `turn` 端口删除。

use symbio_kernel::dialog::{
    Verdict, REASON_ACK, REASON_CLARIFY, REASON_EMPTY, REASON_FROM_CONTEXT, REASON_GREETING,
    REASON_NEEDS_WORK, REASON_REFUSE, REASON_THANKS, REASON_UNCLASSIFIED,
};
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::store::{AppendError, Store};

use crate::control_decl::{EVENT_LOOP_STEP, EVENT_TURN_STAGE};

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

// 八步的**名字**不在这里：它们是行表的数据（`loop_rows::STEP_DECIDE` 等），
// 由本模块的调用方当实参传进来。事实层只认「这一步已发生」+ 一个不透明的 `id`，
// 不持有任何闭集（DE-048 D-321 的「环不认世界名」同一条口径）。

/// 一次判决的事实内容：理由码 + 它由谁产出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerdictFact {
    /// 理由码（`Verdict::Answered` / `Escalate` 的 `reason`；`None` = 没有判决）。
    pub(crate) reason: Option<String>,
    pub(crate) source: VerdictSource,
}

/// `Answered` 答话的全部入格内容——收束格（`chat.assistant.final`）的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnswerFact {
    pub(crate) utterance: String,
    pub(crate) reply: String,
    pub(crate) cost_ms: u64,
    pub(crate) dialog_id: String,
    pub(crate) dialog_reason: String,
    pub(crate) exclude_from_context: bool,
    pub(crate) reasoning: Option<(String, String)>,
}

/// `Escalate` 首响的落格内容（根级对话面节点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpeningFact {
    pub(crate) text: String,
    pub(crate) dialog_id: String,
    pub(crate) dialog_reason: String,
}

/// 一格待落 `loop.step` 的草稿。
///
/// 由阶段执行点（`chat_loop` / `chat_loop::inputs`）产出，落格在持写者令牌的
/// 运行器一侧——与 `ReportDraft` / `DialogFacts` 同一条分割：判定与措辞在编排层，
/// 落格在 `turn_exec`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StageFact {
    /// 这一步的 id（行表数据，见 `loop_rows` 的 `STEP_*`）。它是不透明载荷的一格，
    /// 不是闭集：事实层不为任何世界概念保留词汇。
    pub(crate) id: &'static str,
    /// 本幕轮次（`loop.step{id:"round"}` 的值；`TurnState::round()` 现读事实）。
    pub(crate) round: usize,
    /// 仅 `classify` 携带；其余阶段 `None`（载荷不带 `reason` / `source`）。
    pub(crate) verdict: Option<VerdictFact>,
    /// 这一步的**结论**：`resume` 的恢复种类、`reflex` 的作答结局、`round` 无结论。
    /// `None` = 这一步没有可记的结论（载荷不带 `result`）。
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
    /// 本幕召回的**逐条结果**（仅 `recall` 携带）：`(记忆事件 seq, 标签)`。
    ///
    /// 它是 `turn.recalled` 视图的数据面——能力包的行按这条名单取自己的债
    /// （`eq tag = "skill"`）。宿主只把召回结果如实摊开，不判「哪条是技能」。
    pub(crate) recalled: Vec<(u64, String)>,
    /// 本幕是不是续写幕（仅 `recall` 携带）。
    ///
    /// 续写幕里「命中技能就快路作答」不成立（D-260 的行作答要新开轮），能力包的
    /// 行读这一格自己把快路关掉——宿主不替它判。
    pub(crate) resumed: bool,
    /// 续写幕要替换的旧子节点（仅 `resume` 携带）：新结果落格后由收幕删除。
    pub(crate) replaces: Option<String>,
    /// 本幕召回的逐条正文（仅 `recall` 携带）：`(seq, 标签, 正文)`。
    ///
    /// 召回读数不再寄存在 `TurnState` 里——它是这一格的载荷，`prepare-turn` 与收束
    /// 都从事实重建（DE-048 D-306）。
    pub(crate) recalled_content: Vec<(u64, String, String)>,
    /// 召回是否降级（仅 `recall` 携带；与 `Retrieval::degraded` 同一格语义）。
    pub(crate) degraded: bool,
    /// `Answered` 的答话（仅 `decide` 携带）：收束格的输入。
    pub(crate) answer: Option<AnswerFact>,
    /// `Escalate` 的首响（仅 `decide` 携带）：接缝落根级对话面节点。
    pub(crate) opening: Option<OpeningFact>,
}

impl StageFact {
    /// 一段没有判决的阶段（`recall` / `compress`）。
    pub(crate) fn ran(id: &'static str, round: usize) -> Self {
        Self {
            id,
            round,
            verdict: None,
            result: None,
            identity: None,
            model_calls: 0,
            recalled: Vec::new(),
            resumed: false,
            replaces: None,
            recalled_content: Vec::new(),
            degraded: false,
            answer: None,
            opening: None,
        }
    }

    /// 带结论的一段（`recall` 的命中 / `resume` 的种类）。
    pub(crate) fn with_result(mut self, result: &str) -> Self {
        self.result = Some(result.to_string());
        self
    }

    /// 续写幕要替换的旧子节点（`resume` 专用；`None` = 无需删除）。
    pub(crate) fn with_replaces(mut self, replaces: Option<String>) -> Self {
        self.replaces = replaces;
        self
    }

    /// 记下 `Answered` 的答话（收束格的全部输入）。
    pub(crate) fn with_answer(mut self, answer: AnswerFact) -> Self {
        self.answer = Some(answer);
        self
    }

    /// 记下 `Escalate` 的首响（接缝要落的根级节点）。
    pub(crate) fn with_opening(mut self, opening: OpeningFact) -> Self {
        self.opening = Some(opening);
        self
    }

    /// 恢复幕：这一轮以哪种恢复动作续写（`resume_allowed` 来源）。
    ///
    /// `identity` = 本幕的序数：同一轮可以先后恢复两次**同类**动作，只按
    /// `(turn, round, stage, kind)` 记会把第二次静默吸收掉（幂等键撞上就会让
    /// `drive_seq` 指回首幕）。序数只进事件 id，不进载荷。
    pub(crate) fn resume(id: &'static str, round: usize, kind: &str, identity: &str) -> Self {
        let mut fact = Self::ran(id, round).with_result(kind);
        fact.identity = Some(identity.to_string());
        fact
    }

    /// 第几次请求（`round` 来源）。
    ///
    /// `identity` = 本幕的锚（`drive_seq`）：同一轮号的续写幕会从 `round = 0`
    /// 重来，只按 `(turn, round, stage)` 记会撞首幕的幂等键，于是撞格取回首幕
    /// 的 seq，本幕的锚指向了错误的位置。
    pub(crate) fn request_round(id: &'static str, round: usize, identity: &str) -> Self {
        let mut fact = Self::ran(id, round);
        fact.identity = Some(identity.to_string());
        fact
    }

    /// 轮首判决：理由码与来源一并入格（`None` = 分类器没给出判决，记 `absent`）。
    ///
    /// `model_calls` 是这次判决打了几次模型（0 = 规则短路 / 没有模型服务）。
    pub(crate) fn classify(
        id: &'static str,
        round: usize,
        verdict: Option<&Verdict>,
        model_calls: u64,
    ) -> Self {
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
            id,
            round,
            verdict: Some(VerdictFact { reason, source }),
            result: None,
            identity: None,
            model_calls,
            recalled: Vec::new(),
            resumed: false,
            replaces: None,
            recalled_content: Vec::new(),
            degraded: false,
            answer: None,
            opening: None,
        }
    }

    /// 压缩格：这一次压缩打了几次模型（D-122 决策 2 / 5）。
    ///
    /// `identity` = 本幕的锚（`drive_seq`）：压缩在**续写幕**会重跑（恢复后事实源
    /// 变了，本轮视图应当重取），而同一性若只按 `(turn, round, stage)` 算，续写幕
    /// 那格会撞首幕的幂等键 ⇒ 载荷（含 `model_calls`）**不落** ⇒ 那一笔模型往返
    /// 在事实层没了数，判据侧对不上账。
    pub(crate) fn compress(
        id: &'static str,
        round: usize,
        model_calls: u64,
        drive_seq: u64,
    ) -> Self {
        let mut fact = Self::ran(id, round);
        fact.model_calls = model_calls;
        fact.identity = Some(drive_seq.to_string());
        fact
    }

    /// 召回格：本幕召回了哪几条、是不是续写幕。
    ///
    /// `recalled` 是**逐条结果**（记忆事件 seq + 标签）——`turn.recalled` 视图的数据面，
    /// 能力包的行按它取自己的债（`eq tag`）。`resumed` 让那一行自己决定快路开不开
    /// （续写幕不走快路，D-260）。
    ///
    /// 同一性带上本幕的锚（`drive_seq`）——与 [`Self::compress`] 同一条理由：召回在
    /// 续写幕会重跑（恢复后事实源变了，本轮视图应当重取），只按 `(turn, round, stage)`
    /// 记就撞首幕那一格 ⇒ 本幕的载荷**不落**。而 `turn.recalled` 只读**本幕锚之后**
    /// 的那一格：撞掉的不是载荷，是判据本身。
    pub(crate) fn recall(
        id: &'static str,
        round: usize,
        drive_seq: u64,
        entries: &[(u64, String, String)],
        resumed: bool,
        degraded: bool,
    ) -> Self {
        let mut fact = Self::ran(id, round);
        fact.identity = Some(drive_seq.to_string());
        fact.recalled = entries
            .iter()
            .map(|(seq, tag, _)| (*seq, tag.clone()))
            .collect();
        fact.recalled_content = entries.to_vec();
        fact.resumed = resumed;
        fact.degraded = degraded;
        fact
    }

    /// 反射档格：这一轮的技能行直接作答（`result` = 宿主的作答结局）。
    ///
    /// 这一格由**宿主**在 `RowStep::Answer` 之后代落（D-260）：执行体只答，不落
    /// 「已答」也不落收束；收束格由 `close-turn` 统一落（D-163）。`turn_head.
    /// reflex_answered` 读它，收束行因此在零模型往返的轮上也开得出账。
    ///
    /// 同一性带上本幕的锚（`drive_seq`）——与 [`Self::recall`] 同一条理由：同一轮
    /// 若因事实源补落重放，撞格取回首幕才是对的语义。
    pub(crate) fn reflex(id: &'static str, round: usize, drive_seq: u64, skill_id: &str) -> Self {
        let mut fact = Self::ran(id, round).with_result(skill_id);
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
    pub(crate) fn report(id: &'static str, round: usize, anchor: u64, result: &str) -> Self {
        let mut fact = Self::ran(id, round).with_result(result);
        fact.identity = Some(anchor.to_string());
        fact
    }

    /// 本幕的请求输入已备好：接缝行的出口（同名文档见 [`StageName::Inputs`]）。
    ///
    /// 同一性带上本幕的锚（`drive_seq`）——与 [`Self::recall`] 同一条理由，但这里
    /// 更硬：续写幕复用轮号与请求轮次，`(turn, round, stage)` 三元组会与首幕
    /// **撞成同一格**，而 `land` 把 `Duplicate` 当成功吞掉 ⇒ 续写幕的准备「做完了」
    /// 这件事根本不进日志（e2e t27 抓到的正是这一条：`prepare 落了零格`）。
    pub(crate) fn inputs(id: &'static str, round: usize, drive_seq: u64, result: &str) -> Self {
        let mut fact = Self::ran(id, round).with_result(result);
        fact.identity = Some(drive_seq.to_string());
        fact
    }

    /// 事件 id：`(turn, id, 幕锚)` 是这一格的**同一性**（见模块文档）。
    ///
    /// 轮号不进 id（它不是键）；每一幕用自己的锚收窄，续写幕因此各落一格。
    pub(crate) fn event_id(&self, turn: u64, anchor: u64) -> String {
        let tail = self.identity.clone().unwrap_or_else(|| anchor.to_string());
        format!("st-{turn}-{}-{tail}", self.id)
    }

    pub(crate) fn payload(&self, anchor: u64) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "id": self.id,
            "anchor": anchor,
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
        // `model_calls` 跟着**数据**走，不跟着步名走：报过这次数的格才写
        // （判决格恒报，0 = 规则短路；压缩格只在真打过时报）。「恒 0 的字段」是噪声。
        if self.verdict.is_some() || self.model_calls > 0 {
            payload["model_calls"] = serde_json::Value::from(self.model_calls);
        }
        // 召回读数同理：有逐条结果、或带了降级 / 续写标记时才写。
        // 它们是 `turn.recalled` 的数据面；缺格由读侧按空 / `false` 兜底。
        if !self.recalled_content.is_empty() || self.resumed || self.degraded {
            payload["recalled"] = serde_json::Value::Array(
                self.recalled_content
                    .iter()
                    .map(|(seq, tag, content)| {
                        serde_json::json!({ "seq": seq, "tag": tag, "content": content })
                    })
                    .collect(),
            );
            payload["resumed"] = serde_json::Value::Bool(self.resumed);
            payload["degraded"] = serde_json::Value::Bool(self.degraded);
        }
        if let Some(answer) = &self.answer {
            payload["answer"] = serde_json::json!({
                "utterance": answer.utterance,
                "reply": answer.reply,
                "cost_ms": answer.cost_ms,
                "dialog_id": answer.dialog_id,
                "dialog_reason": answer.dialog_reason,
                "exclude_from_context": answer.exclude_from_context,
                "reasoning_id": answer.reasoning.as_ref().map(|(id, _)| id),
                "reasoning": answer.reasoning.as_ref().map(|(_, text)| text),
            });
        }
        if let Some(opening) = &self.opening {
            payload["opening"] = serde_json::json!({
                "text": opening.text,
                "dialog_id": opening.dialog_id,
                "dialog_reason": opening.dialog_reason,
            });
        }
        // 续写幕的旧子节点：收幕按它删除被替换的旧卡（`None` = 不写这一格）。
        if let Some(replaces) = &self.replaces {
            payload["replaces"] = serde_json::Value::String(replaces.clone());
        }
        payload
    }
}

/// 本幕锚之后最近落下的某一步（`None` = 还没做过）。
///
/// 读侧只认**事实**：`TurnState` 里不再寄存这些步值（DE-048 D-306）。
pub(crate) fn latest_step<'a>(
    events: &'a [Event],
    turn: u64,
    anchor: u64,
    id: &str,
) -> Option<&'a Event> {
    events
        .iter()
        .filter(|e| {
            e.scope == turn
                && e.kind == EVENT_LOOP_STEP
                && e.seq.map(|s| s.value()).is_some_and(|seq| seq >= anchor)
                && e.payload.get("id").and_then(serde_json::Value::as_str) == Some(id)
        })
        .max_by_key(|e| e.seq.map(|s| s.value()))
}

/// 本幕的请求轮次（`loop.step{id:"round"}` 的值；取不到 = 首幕尚未落格）。
pub(crate) fn round_of(events: &[Event], turn: u64, anchor: u64) -> Option<usize> {
    latest_step(events, turn, anchor, crate::loop_rows::STEP_ROUND)?
        .payload
        .get("round")
        .and_then(serde_json::Value::as_u64)
        .map(|round| usize::try_from(round).unwrap_or(usize::MAX))
}

/// 召回格的读数：逐条正文 + 降级标记。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RecallRecord {
    pub(crate) entries: Vec<(u64, String, String)>,
    pub(crate) degraded: bool,
}

/// 本幕的召回读数。
pub(crate) fn recall_of(events: &[Event], turn: u64, anchor: u64) -> Option<RecallRecord> {
    let fact = latest_step(events, turn, anchor, crate::loop_rows::STEP_RECALL)?;
    let entries = fact
        .payload
        .get("recalled")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some((
                        item.get("seq")?.as_u64()?,
                        item.get("tag")?.as_str()?.to_string(),
                        item.get("content")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let degraded = fact
        .payload
        .get("degraded")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Some(RecallRecord { entries, degraded })
}

/// 判决的结论是不是「本轮就此收尾」（`loop.step{id:"decide"}` 的 `result`）。
///
/// **不按幕锚过滤**：同一个轮里出现一格 `answered` 就意味着该轮已经收束，后面不可能
/// 再续一幕；按锚过滤反而会把「先落决定格、后开用户格」的测试替身挡在外面。
pub(crate) fn answered(events: &[Event], turn: u64, _anchor: u64) -> bool {
    latest_step(events, turn, 0, crate::loop_rows::STEP_DECIDE).is_some_and(|fact| {
        fact.payload
            .get("result")
            .and_then(serde_json::Value::as_str)
            == Some("answered")
    })
}

/// `Answered` 答话的事实内容（收束格输入）。
pub(crate) fn answer_of(events: &[Event], turn: u64, _anchor: u64) -> Option<AnswerFact> {
    let fact = latest_step(events, turn, 0, crate::loop_rows::STEP_DECIDE)?;
    let answer = fact.payload.get("answer")?;
    let reasoning = match (
        answer
            .get("reasoning_id")
            .and_then(serde_json::Value::as_str),
        answer.get("reasoning").and_then(serde_json::Value::as_str),
    ) {
        (Some(id), Some(text)) => Some((id.to_string(), text.to_string())),
        _ => None,
    };
    Some(AnswerFact {
        utterance: answer.get("utterance")?.as_str()?.to_string(),
        reply: answer.get("reply")?.as_str()?.to_string(),
        cost_ms: answer
            .get("cost_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        dialog_id: answer
            .get("dialog_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        dialog_reason: answer
            .get("dialog_reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        exclude_from_context: answer
            .get("exclude_from_context")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        reasoning,
    })
}

/// `Escalate` 首响的事实内容（接缝落根级节点）。
pub(crate) fn opening_of(events: &[Event], turn: u64, anchor: u64) -> Option<OpeningFact> {
    let fact = latest_step(events, turn, anchor, crate::loop_rows::STEP_DECIDE)?;
    let opening = fact.payload.get("opening")?;
    Some(OpeningFact {
        text: opening.get("text")?.as_str()?.to_string(),
        dialog_id: opening
            .get("dialog_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        dialog_reason: opening
            .get("dialog_reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
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

/// 把一批阶段草稿落成 `loop.step`（在幕锚之后调用——I2 要求断言类事件带溯源）。
///
/// `Duplicate` 不是错误：同一 `(turn, id, 幕锚)` 已落意味着这一步做过，
/// 续写幕重跑同一幕时会命中（见模块文档）。
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

/// 本幕的幕锚：本 `turn` 内最新的驱动锚（`user.message` / `loop.step{id:"resume"}` /
/// `loop.step{id:"round", round>0}`）的 seq。
///
/// `land` 用它写载荷 `anchor` 与事件 id——锚是**幕**的属性，不是调用方手里的
/// 溯源号：续写幕的 `user_seq` 仍是原轮用户格（I2 要求 `produced_by` 记它），
/// 但幕锚已经挪到恢复格上。
fn scene_anchor<S>(store: &S, turn: u64) -> Option<u64>
where
    S: Store<Event = Event> + ?Sized,
{
    // 幕锚的判据只有一个 owner（`control_decl::is_drive_anchor`）：本模块不另写一份
    // 「什么样的格子算锚」——那会是同一判据的第二个版本。
    store
        .range(symbio_kernel::event::Seq::new(0))
        .into_iter()
        .filter(|e| e.scope == turn && crate::control_decl::is_drive_anchor(e))
        .filter_map(|e| e.seq.map(|s| s.value()))
        .max()
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
    let anchor = scene_anchor(store, turn).unwrap_or(user_seq);
    let mut seqs = Vec::with_capacity(stages.len());
    for stage in stages {
        let id = stage.event_id(turn, anchor);
        let event = Event::pending(
            id.clone(),
            EVENT_LOOP_STEP,
            Entity::Scope,
            Verb::Asserted,
            turn,
            actor,
        )
        .with_produced_by(user_seq)
        .with_payload(stage.payload(anchor));
        match store.append(event) {
            Ok(seq) => seqs.push(seq.value()),
            Err(AppendError::Duplicate) => {
                let existing = store
                    .range(symbio_kernel::event::Seq::new(0))
                    .into_iter()
                    .find(|e| e.scope == turn && e.kind == EVENT_LOOP_STEP && e.event_id == id)
                    .and_then(|e| e.seq.map(|s| s.value()))
                    .unwrap_or(0);
                seqs.push(existing);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(seqs)
}

/// 能力交来的阶段效果（`D-293`）：名字由能力给，身份由宿主按
/// `st-{anchor}-{stage}` 定。
///
/// 它仍住在**本模块**：`turn.stage` 的写法只有一处（`D-228`），效果面只是换了
/// 身份的拼法（能力拿不到轮号，只能拿宿主给的不透明词干）。`Duplicate` 与
/// [`land`] 同一条口径——重试 / 续写幕重复投递同一份草稿不算失败。
pub(crate) fn land_effect<S>(
    store: &S,
    turn: u64,
    user_seq: u64,
    actor: &str,
    anchor: &str,
    stage: &str,
    round: usize,
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let event = Event::pending(
        format!("st-{anchor}-{stage}"),
        EVENT_TURN_STAGE,
        Entity::Scope,
        Verb::Asserted,
        turn,
        actor,
    )
    .with_produced_by(user_seq)
    .with_payload(serde_json::json!({ "stage": stage, "round": round }));
    match store.append(event) {
        Ok(_) | Err(AppendError::Duplicate) => Ok(()),
        Err(e) => Err(e),
    }
}

/// 🔴 这一步在事实面上是否**已经做过**（按轮折叠，测试用）。
///
/// ⚠️ 它扫全史：续写幕复用同一轮号，首幕落过的格会被判成"已做"，而续写幕的语义
/// 恰恰是「恢复后事实源变了，本轮视图应重取」（召回 / 压缩重跑）。生产侧读的是
/// `turn_head` 视图的按幕锚折叠——本函数只钉住「事实 → 已做」这个形状。
#[cfg(test)]
pub(crate) fn done(events: &[Event], turn: u64, id: &str, round: usize) -> bool {
    events.iter().any(|e| {
        e.kind == EVENT_LOOP_STEP
            && e.scope == turn
            && e.verb == Verb::Asserted
            && e.payload.get("id").and_then(|v| v.as_str()) == Some(id)
            && e.payload.get("round").and_then(|v| v.as_u64()) == Some(round as u64)
    })
}

#[cfg(test)]
#[path = "stage_events.test.rs"]
mod tests;
