//! 对话面契约 —— `session` ⇄ `classify` / `compose` 之间唯一的类型面
//!
//! ## 为什么在 core
//!
//! 契约的生产方是 `session`（它投影事实、发起调用），消费方是 `classify`（判决）与
//! `compose`（措辞）**两个**插件。依赖方 ≥ 2 且分处不同插件 ⇒ 满足 `D-023` 的
//! core 准入判据：插件之间不得互相 import（`plugin-entry-audit` 的 E-009 在守），
//! 共享类型只能落在 core。
//!
//! ## 一条判据：判决是**枚举**，措辞是**文本**
//!
//! [`Verdict`] 是闭集枚举，不是自由文本。理由不是「枚举更好看」——是**编排层要能
//! 执行它**：`Answered` 要收尾、`Escalate` 要进工具循环、`Report` 只是说一句话。
//! 让模型返回一段文本再由编排层解析，等于把「这轮走哪条路」变成一次不可靠的
//! 字符串匹配（部分协议下还会因连续两条 `assistant` 直接 400）。
//!
//! 反过来，[`ComposeRequest`] 的产出是 `String`：措辞只被展示，不被执行。
//! **判决不做措辞，措辞不做判决。**
//!
//! ## 理由码是 `String`，词汇表与本契约同处
//!
//! [`Verdict::Answered`] / [`Verdict::Escalate`] 带的 `reason` 是**理由码**（下方
//! `REASON_*`），用来决定措辞走模板还是走生成。**字段**是 `String` 而不是枚举：
//! 新增一类理由只加一行常量，`Verdict` 的**变体**不变（J1 —— 能用已有数据的取值
//! 表达的，就不是机制）。而**词汇表**住在 core，因为它生产方与消费方分处两个插件
//! （`classify` 产码、`compose` 按码选措辞），满足 `D-023` 的 core 准入判据。
//! 同一份线上词表在两个插件各写一份，漂移的表现是**用户收到一句通用兜底话**——
//! 功能还在、没有错误信号，而那正是必须外置成断言的那类失效；比断言更省的是让它
//! 没有可漂的第二处。这与 `meta` 是自由 JSON 是同一条取舍。
//!
//! ## 运行现状（[`RunSnapshot`]）：**只放事实，不放判断**
//!
//! `Report` 的措辞要从"现在到哪一步了"组织出来，而那是 `session` 在**调用那一刻**
//! 才知道的事实。因此它随请求带来，**不由插件去收集**——一旦这里出现一个判断字段
//! （`needs_work: bool`…），措辞方就被赋予了决策权，而决策权必须留在有工具、
//! 有上下文的编排层。
//!
//! ### 为什么它只有两个字段
//!
//! 只收**当下有生产者**的事实。设计里还列过「正在跑什么」「失败原因」「计划」——
//! 汇报的判定点在**轮边界**（一轮工具刚跑完、准备发下一次请求），那一刻：
//! 没有在跑的工具（批已收齐）、没有失败（失败会让本轮直接收尾）、没有计划
//! （计划归模型，不在编排层手里）。写进来就是替未来的实现猜形状。
//!
//! ### 为什么 `ComposeRequest` 不带字数上限
//!
//! 答话的「短」由**提示词**约束，不由硬截断——截断会把句子切一半，比长一句更糟。
//! 而 `session` 并没有比 `compose` 更好的字数知识：它唯一能给的那个数只能是从别处
//! 抄来的常量。一个没有生产者的字段就是预留，不因为它是 `Option` 就例外。
//!
//! ## 新增字段为什么**不带** `deny_unknown_fields` 且带 `#[serde(default)]`
//!
//! 本模块的请求还有**第二个调用方**：网关把外部客户端的 `path` 原样转发给容器
//! （`gateway/server.rs`），因此 `classify/decide` 也可能由仓外程序直接调用。
//! 缺省值让「老调用方不传新字段」与「新调用方不传可选字段」都成立——
//! 契约扩展不产生破坏性变更。

use serde::{Deserialize, Serialize};

use crate::chat_node::ChatNode;
use crate::projection::transcript::PromptMessage;

/// `classify` 的入参：这一轮发生了什么
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecideRequest {
    /// 会话 id（插件据此取自己的上下文；两个插件都不持有会话状态）
    pub session_id: String,
    /// 用户这一轮说的话；无用户输入时为 `None`（如后台触发的判定）
    #[serde(default)]
    pub utterance: Option<String>,
    /// **对话线**投影（`session` 经 `context::conversation_view` 投影后随请求带来）。
    ///
    /// ## 为什么是投影而不是会话 id 自取
    ///
    /// 转写只有一个写入者（D-020），读侧同理只有一个**投影**入口：`classify`
    /// 不读存储、不认节点树，它拿到的是「用户与助手说过的话」这一条线——
    /// 判「已知事实能否直答」只需要这条线。
    ///
    /// ## 为什么这个字段现在才加
    ///
    /// 它要有生产者才有意义（`conversation_view` 建立之后才有值），所以字段与
    /// 投影函数同时落位，不替未来猜形状。
    #[serde(default)]
    pub context: Vec<ChatNode>,
}

/// `classify` 的**判决结果**：判决本身，或一次**待跑的模型往返**。
///
/// ## 为什么不是一个 `Verdict`
///
/// D-122 决策 2：判决的模型往返走**引擎的 `model` 端口派发路径**，插件侧不再
/// 保留直接打模型的入口。于是「判不判得出来」与「要不要打模型」在调用前是**两件事**
/// ——规则表命中时压根没有模型往返（决策 4：零往返 = 行不成立）。
///
/// 契约把这两件事分成两个变体，编排层据此决定**要不要派发**：
///
/// - [`DecideOutcome::Decided`]：已经判完（规则短路 / 本轮没有用户发言）——零入账格；
/// - [`DecideOutcome::NeedsModel`]：请引擎打一次模型，附一份**计划**。
///
/// ## 为什么计划走契约，而不是插件自己去打
///
/// 「这一轮欠一次模型往返」是**编排层的判断**（DE-015 §2.3：`request` 由决策者写），
/// 而模型怎么打是**能力的内部知识**（提示词、消息怎么拼）。两者分处两个插件 ⇒ 计划
/// 只能经契约传递。计划是**数据**：编排层拿到它不必懂分类，只需照着打。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DecideOutcome {
    /// 判完了（规则短路命中 / 本轮没有用户发言）：**零模型往返**。
    Decided {
        /// 判决（闭集枚举）。
        verdict: Verdict,
    },
    /// 需要一次模型往返：编排层按 [`DecisionPlan`] 派发，把应答交回 `classify/parse`。
    NeedsModel {
        /// 这一次往返的**计划**。
        plan: DecisionPlan,
    },
}

/// 一次判决模型往返的**计划**（`classify` 备好，编排层执行）。
///
/// 三个字段都是**执行所需的全部**：提示词、消息、模型。三者缺一，编排层就得反过来
/// 猜——而「猜」正是 D-122 否掉的候选 ②（调用前问「会不会打模型」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionPlan {
    /// 系统提示词（内置那份或本插件配置覆盖的那份）。
    pub system_prompt: String,
    /// 消息（对话线投影 + 本轮这句话）。
    pub messages: Vec<PromptMessage>,
    /// 判决用的 provider id（`<根>/model/<id>`）。
    ///
    /// `None` = 用**会话选定**的那一个（配置的 id 写错时也落回它——降级而不失效）。
    #[serde(default)]
    pub model_id: Option<String>,
}

/// `classify/parse` 的入参：把分类器的**自由文本**解析成判决。
///
/// ## 为什么解析是**另一次路由**，而不是回参里带一句
///
/// 判决是闭集枚举，而模型的输出是自由文本（提示词要求只输出一个词，模型不总听话）。
/// 把「解析」留在 `classify` 侧，编排层就只认枚举——它不需要知道四个词是哪四个
/// （词表是 `classify` 的内部知识）。文本经容器路由回 `classify`，与判决同一个插件
/// 拥有同一份词表 ⇒ 词表只有一处。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseVerdictRequest {
    /// 会话 id（插件据此取自己的上下文）。
    pub session_id: String,
    /// 分类器的应答正文（自由文本）。
    pub text: String,
}

/// 编排层要**执行**的那个枚举：这一轮该走哪条路。
///
/// 三个变体对应三种已确认的诉求——能直接答的直接答、需要干活的派给工具循环、
/// 干到一半也要能说一句话。**它是闭集**：新增一条"走哪条路"是编排语义的变化，
/// 不是加一个取值。
///
/// ## 三个变体的产出方不是同一个
///
/// | 变体 | 产出方 | 为什么是它 |
/// |---|---|---|
/// | `Answered` / `Escalate` | `classify` | 判「用户这句话要不要派活」需要**语义**判断，而 `classify` 是那个能力 |
/// | `Report` | `session` | 「该不该汇报」是**会话状态**上的判定（静默多久、跑了几轮、说过几次），`classify` 手里没有这些量；把它交给 `classify` 等于让它对一个已经定好的结论盖章 |
///
/// 因此本枚举是**编排层的执行面**，不是某个插件的私有出参：`session` 既能执行
/// 别人判出来的结果，也能自己判出 `Report`。两者对 `compose` 是同一种输入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// 能直接答（不需要工具）：`reason` 决定措辞走模板还是走生成
    Answered {
        /// 理由码（数据，见模块文档）
        reason: String,
    },
    /// 需要干活：进工具循环；`reason` 供措辞选首响
    Escalate {
        /// 理由码（数据，见模块文档）
        reason: String,
    },
    /// 该汇报了：措辞由 `compose` 从**运行现状**（[`RunSnapshot`]）组织
    Report,
}

// ── 理由码词表：上面两个变体的 `reason` 取值 ──────────────────────────────
//
// 新增一类理由 = 在这里加一行常量，再去 `classify` 产出它、去 `compose` 给它措辞。
// 三步都不动类型，因此它们都是**数据**；只有「这一轮多一条可执行的路」才是机制，
// 而那要动 [`Verdict`] 的变体。

/// 问候（「你好」「hello」…）
pub const REASON_GREETING: &str = "greeting";
/// 致谢（「谢谢」「thanks」…）
pub const REASON_THANKS: &str = "thanks";
/// 确认 / 收到（「好的」「ok」…）
pub const REASON_ACK: &str = "ack";
/// 空输入（只有空白 / 标点）
pub const REASON_EMPTY: &str = "empty";

/// 问的是**已有上下文**里的事实，能直接答 —— `compose` 侧唯一的**生成**产线
/// （模板给不了「我们刚才聊了什么」的答案）
pub const REASON_FROM_CONTEXT: &str = "from_context";
/// 话没说清，需要反问一句
pub const REASON_CLARIFY: &str = "clarify";
/// 不该做 / 做不了，需要明确拒绝
pub const REASON_REFUSE: &str = "refuse";
/// 要干活，派给工具循环 —— `Escalate` 的首响（一句「我来处理」，不等判决之外的任何东西）
pub const REASON_NEEDS_WORK: &str = "needs_work";

/// **判不出来**（没有可用的模型服务 / 分类响应不可解析 / 本轮没有用户发言）。
///
/// 兜底方向是 `Escalate`，不是 `Answered`：后者会让「分类器坏了」表现成「这轮不用
/// 干活」，用户看到的是**沉默**——这里最坏的失败形态，因为它没有任何错误信号。
pub const REASON_UNCLASSIFIED: &str = "unclassified";

impl Verdict {
    /// 这条判决要不要花**一次模型往返**来组织措辞。
    ///
    /// 判据只有一个理由码：`from_context`——「答案已在对话里」，那段文本只能从对话线
    /// 组织出来，模板给不了；其余理由码的**内容**是固定的，模型只会把它们写长。
    ///
    /// 它住在契约上而不是 `compose` 的私有函数里，因为读它的不止措辞那一侧：编排层
    /// 要按它决定「这一次对外调用是不是一笔端口 `model` 的账」（DE-015 判据 2 要求
    /// **每一次**模型往返都有 `request` / `sent` / `response` 三格，内部静默请求不
    /// 例外）。两处各判一遍必然漂移成「记了账却没打模型」或「打了模型没账」。
    pub fn needs_wording_generation(&self) -> bool {
        matches!(self, Verdict::Answered { reason } if reason == REASON_FROM_CONTEXT)
    }
}

/// **运行现状快照** —— `session` 在调用那一刻投影出来的事实（只放事实，见模块文档）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSnapshot {
    /// 本轮**已完成**的工具轮次。
    #[serde(default)]
    pub tool_rounds: usize,
    /// **对话静默时长**（毫秒）：距对话线上最近一次动静（用户发言 / 助手说话）。
    ///
    /// 它就是「用户等了多久」。不是「本轮已用时」——后者的起点是轮次开始，而
    /// 用户感知到的是"上次有人说话之后过了多久"；两者在轮首判决写入首响时会分叉。
    #[serde(default)]
    pub quiet_ms: i64,
}

/// `compose` 的入参
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeRequest {
    /// 会话 id
    pub session_id: String,
    /// 上游判决结果 —— 措辞**执行**判决，不重新判决
    pub verdict: Verdict,
    /// **对话线**投影（与 [`DecideRequest::context`] 同一份规则、同一个函数）。
    ///
    /// ## 为什么措辞需要它
    ///
    /// `Answered { reason: "from_context" }` 的含义是「答案已在对话里」——那段文本
    /// 只能从对话线**组织**出来，模板给不了。其余理由码走模板，用不到这个字段，
    /// 但字段不按分支可选：让 `compose` 的入参形状随判决变，等于把编排细节泄进契约。
    #[serde(default)]
    pub context: Vec<ChatNode>,
    /// **运行现状**（[`RunSnapshot`]）。`Report` 的措辞据此组织。
    ///
    /// 与 `context` 同一条纪律：字段不按分支可选。`Answered` / `Escalate` 用不到它
    /// （它们说的是"这一轮"的事，而现状说的是"干到哪一步"），但让入参形状随判决变
    /// 就会让 `compose` 的分派从"看判决"退化成"看字段在不在"。
    #[serde(default)]
    pub snapshot: RunSnapshot,
}

/// `compose/wording` 的**出参**。
///
/// ## 为什么不再是一个裸 `String`
///
/// 答话由一次**静默 LLM 往返**生成（`from_context` 产线），那次往返的思考内容
/// （`reasoning` / `reasoning_id`）此前在措辞插件内部就被丢弃——收束格只入格了
/// 答话文本，`Reason` 节点对普通答话轮从未存在过（AMD MiMo 实测 2026-10-13，
/// 接上字段兼容修复后流层明明收到了推理分片）。模板产线没有模型往返，
/// 两个推理字段恒 `None`、`cost_ms` 恒 `0`。
///
/// 字段全部 `Option` + `default`：模板路径的反序列化不需要协作。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordingReply {
    /// 答话文本（对话面节点正文）。
    pub text: String,
    /// 生成这次答话的思考内容（模板路径为 `None`）。
    #[serde(default)]
    pub reasoning: Option<String>,
    /// 思考子节点 id（与 `reasoning` 同源；`None` = 没有思考）。
    #[serde(default)]
    pub reasoning_id: Option<String>,
    /// 这次措辞往返在 adapter 边界的**实测耗时**（毫秒；模板路径为 `0`）。
    ///
    /// 它是**收束格的 `cost_ms` 来源**：`Answered` 轮不进驱动循环，收束格由
    /// `execute_final_reply_turn` 就地落——这一次措辞调用是该轮唯一的模型往返，
    /// 不把它带回来，`chat.assistant.final` 就恒记 `0`，熔断与 SLO 的台账里
    /// 这一整类轮次的成本凭空消失。
    #[serde(default)]
    pub cost_ms: u64,
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
