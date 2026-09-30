//! 理由码词表 —— [`Verdict`] 的 `reason` 取值。
//!
//! ## 为什么是常量而不是枚举
//!
//! 理由码是**数据**（J1：能用已有参数的新取值表达的，就不是机制）。新增一类理由
//! 只在本文件加一行，不动 `symbio_core`——`Verdict` 的**变体**是闭集（编排语义），
//! 它的 `reason` 取值是**开放词汇表**（措辞策略），两者刻意不同级。
//!
//! ## owner 与消费方
//!
//! 生产方是本插件；消费方是 `compose`（按码选模板 / 决定生成）。两侧**不共享常量**：
//! 共享常量会把「加一行数据」升级成「改 core」。代价是码表可能在两侧漂移，
//! 兜底是 `compose` 对未知码走通用模板——**降级而不失效**，这正是数据面允许的松弛度。
//!
//! ## 两条产线
//!
//! | 产线 | 文件 | 代价 |
//! |---|---|---|
//! | 规则表（反射档） | [`super::rules`] | 0 次 LLM 往返 |
//! | 快速档分类 | [`super::classify`] | 1 次**静默** LLM 往返 |

/// 问候（「你好」「hello」…）
pub(crate) const REASON_GREETING: &str = "greeting";
/// 致谢（「谢谢」「thanks」…）
pub(crate) const REASON_THANKS: &str = "thanks";
/// 确认 / 收到（「好的」「ok」…）
pub(crate) const REASON_ACK: &str = "ack";
/// 空输入（只有空白 / 标点）
pub(crate) const REASON_EMPTY: &str = "empty";

/// 快速档：问的是**已有上下文**里的事实，能直接答
pub(crate) const REASON_FROM_CONTEXT: &str = "from_context";
/// 快速档：话没说清，需要反问一句
pub(crate) const REASON_CLARIFY: &str = "clarify";
/// 快速档：不该做 / 做不了，需要明确拒绝
pub(crate) const REASON_REFUSE: &str = "refuse";
/// 快速档：要干活，派给工具循环
pub(crate) const REASON_NEEDS_WORK: &str = "needs_work";

/// **判不出来**（没有可用的模型服务 / 分类响应不可解析 / 本轮没有用户发言）。
///
/// 兜底方向是 `Escalate`——「拿不准就当要干活」，与未挂载本插件时的行为一致。
/// 兜底不能是 `Answered`：那会让「分类器坏了」表现成「这轮不用干活」，
/// 用户看到的是**沉默**，而沉默是这里最坏的失败形态（没有任何错误信号）。
pub(crate) const REASON_UNCLASSIFIED: &str = "unclassified";
