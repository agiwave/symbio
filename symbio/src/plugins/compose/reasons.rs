//! 理由码词表 —— [`Verdict`] 的 `reason` 取值（**本插件侧的抄本**）。
//!
//! ## 为什么是抄本而不是共享常量
//!
//! 生产方是 `classify`。两侧**不共享常量**：常量要共享就必须住在 `symbio_core`，
//! 那会把「加一行数据」升级成「改 core」——而理由码是**数据**（J1：能用已有参数的
//! 新取值表达的，就不是机制）。
//!
//! 代价是两侧可能漂移。漂移的**兜底**是本插件对未知码走通用模板
//! （见 [`super::templates`]）：新增一类理由而忘了在这里配措辞，用户收到的是一句
//! 通用话，而不是**空白**——降级而不失效，这正是数据面允许的松弛度。
//!
//! 但兜底不等于可以不守：`compose/templates.test.rs` 里有一条**抄本一致性**用例，
//! 直接拿 `classify` 的词表比对本表（生产代码里两个插件仍互不可见，`E-009` 在守；
//! 跨插件可见性**只为这条测试**放开一个模块，见 `classify/mod.rs` 的说明）。
//!
//! ## 本文件的值必须与 `plugins/classify/reasons.rs` 逐字一致
//!
//! 它们不是各自的私有取值，是**同一份线上词汇表的两份抄本**。

/// 问候（「你好」「hello」…）—— 模板
pub(crate) const REASON_GREETING: &str = "greeting";
/// 致谢（「谢谢」「thanks」…）—— 模板
pub(crate) const REASON_THANKS: &str = "thanks";
/// 确认 / 收到（「好的」「ok」…）—— 模板
pub(crate) const REASON_ACK: &str = "ack";
/// 空输入（只有空白 / 标点）—— 模板
pub(crate) const REASON_EMPTY: &str = "empty";

/// 问的是**已有上下文**里的事实 —— **生成**（模板给不了"我们刚才聊了什么"的答案）
pub(crate) const REASON_FROM_CONTEXT: &str = "from_context";
/// 话没说清，需要反问一句 —— 模板
pub(crate) const REASON_CLARIFY: &str = "clarify";
/// 不该做 / 做不了，需要明确拒绝 —— 模板
pub(crate) const REASON_REFUSE: &str = "refuse";

/// 要干活 —— `Escalate` 的**首响**模板（一句"我来处理"，不等判决之外的任何东西）
pub(crate) const REASON_NEEDS_WORK: &str = "needs_work";
/// 判不出来 —— `Escalate` 的首响模板（与 `needs_work` 不同措辞：一个"我确认过要干活"，
/// 一个"我还没看清，先动手看看"）
pub(crate) const REASON_UNCLASSIFIED: &str = "unclassified";
