//! 对话面契约 —— `session` ⇄ `triage` / `reply` 之间唯一的类型面
//!
//! ## 为什么在 core
//!
//! 契约的生产方是 `session`（它投影事实、发起调用），消费方是 `triage`（判决）与
//! `reply`（措辞）**两个**插件。依赖方 ≥ 2 且分处不同插件 ⇒ 满足 `ADR-023` 的
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
//! ## 本模块刻意不定义的字段
//!
//! 两个请求都**只带当下真实存在的东西**。运行现状快照（在跑什么工具、已用时、
//! 失败原因）要等 `session` 侧把它投影出来才有生产者——先写字段等于替未来的实现
//! 猜形状。它各自在需要的那一刻加进来（新增字段是 `L-schema` 级变化，生产者与
//! 消费者同仓，编译期就会拦住漏改）。
//!
//! ## 新增字段为什么**不带** `deny_unknown_fields` 且带 `#[serde(default)]`
//!
//! 本模块的请求还有**第二个调用方**：网关把外部客户端的 `path` 原样转发给容器
//! （`gateway/server.rs`），因此 `triage/decide` 也可能由仓外程序直接调用。
//! 缺省值让「老调用方不传新字段」与「新调用方不传可选字段」都成立——
//! 契约扩展不产生破坏性变更。

use serde::{Deserialize, Serialize};

use super::session::chat_message::ChatMessage;

/// `triage` 的入参：这一轮发生了什么
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
    /// 转写只有一个写入者（ADR-020），读侧同理只有一个**投影**入口：`triage`
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

/// `triage` 的出参：这一轮该走哪条路
///
/// 三个变体对应三种已确认的诉求——能直接答的直接答、需要干活的派给工具循环、
/// 干到一半也要能说一句话。**它是闭集**：新增一条"走哪条路"是编排语义的变化，
/// 不是加一个取值。
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
    /// 该汇报了：措辞由 `reply` 从运行现状组织
    Report,
}

/// `reply` 的入参
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeRequest {
    /// 会话 id
    pub session_id: String,
    /// 上游判决结果 —— 措辞**执行**判决，不重新判决
    pub verdict: Verdict,
    /// 字数上限；`None` = 不限制
    pub max_chars: Option<usize>,
}
