//! 对话面契约 —— `session` ⇄ `classify` / `compose` 之间唯一的类型面
//!
//! ## 为什么在 core
//!
//! 契约的生产方是 `session`（它投影事实、发起调用），消费方是 `classify`（判决）与
//! `compose`（措辞）**两个**插件。依赖方 ≥ 2 且分处不同插件 ⇒ 满足 `ADR-023` 的
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
//! ## 理由码是 `String`，不是枚举
//!
//! [`Verdict::Answered`] / [`Verdict::Escalate`] 带的 `reason` 是**理由码**（如
//! `"greeting"` / `"from_context"`），用来决定措辞走模板还是走生成。它是**数据**：
//! 新增一类理由只加一行码表，不动 core（J1 —— 能用已有参数的新取值表达的，
//! 就不是机制）。这与 `meta` 是自由 JSON 是同一条取舍。
//!
//! ## 运行现状（[`RunSnapshot`]）：**只放事实，不放判断**
//!
//! `Report` 的措辞要从"现在到哪一步了"组织出来，而那是 `session` 在**调用那一刻**
//! 才知道的事实。因此它随请求带来，**不由插件去收集**——一旦这里出现一个判断字段
//! （`needs_work: bool`…），措辞方就被赋予了决策权，而决策权必须留在有工具、
//! 有上下文的编排层（见 `docs/archive/chat-worker-split.md` §3.3）。
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

use super::session::chat_message::ChatMessage;

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
    /// 转写只有一个写入者（ADR-020），读侧同理只有一个**投影**入口：`classify`
    /// 不读存储、不认节点树，它拿到的是「用户与助手说过的话」这一条线——
    /// 判「已知事实能否直答」只需要这条线。
    ///
    /// ## 为什么这个字段现在才加
    ///
    /// S1 时它没有生产者（`conversation_view` 尚未建立），写了就是替未来猜形状。
    /// S2 里投影函数与它的第一个消费方同批落位，字段与生产者同时出现。
    #[serde(default)]
    pub context: Vec<ChatMessage>,
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
    pub context: Vec<ChatMessage>,
    /// **运行现状**（[`RunSnapshot`]）。`Report` 的措辞据此组织。
    ///
    /// 与 `context` 同一条纪律：字段不按分支可选。`Answered` / `Escalate` 用不到它
    /// （它们说的是"这一轮"的事，而现状说的是"干到哪一步"），但让入参形状随判决变
    /// 就会让 `compose` 的分派从"看判决"退化成"看字段在不在"。
    #[serde(default)]
    pub snapshot: RunSnapshot,
}
