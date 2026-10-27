//! 行为名 —— 工具对自己的**能力效果**做的自声明（DE-025 D-210）。
//!
//! 与 `Capability` 同层：能力包在这里取一个名字写进 `CapabilityMeta.behaviors`，
//! 宿主侧的闸门按名字判，**不认工具名**。新增一个行为**实例**（「我的工具也写
//! 任务表」）= 在这里加一个常量 + 在工具里声明 + 注册 handler；新增一种行为
//! **语义**（框架第一次出现这种判断）才是架构变更。
//!
//! 内核只装开放集合（`&'static [&'static str]`），不认识下面任何名字——所以
//! 加一个行为不再改 `symbio-kernel`。已知名字留在本模块的理由与 `routes` /
//! `events` 相同：它们是宿主词汇，不是内核语法。
//!
//! ## 已知行为
//!
//! | 常量 | 线格式 | 管辖方 |
//! |---|---|---|
//! | [`ASSIGN_WORK`] | `assign_work` | 淘汰闸：`assign.work` 被收回时拒止 |
//! | [`DELEGATION_PROMISE`] | `delegation_promise` | 承诺记账：跨主体立约的出参 |
//! | [`TASK_TABLE`] | `task_table` | 任务表写侧资格闸与任务声明记账 |
//!
//! 名字是**跨进程的**（工具元数据随插件通道走），改名等于改协议；只在语义真的要
//! 变的时候改，并在 `DE-025` 留一条决策。

/// 提供 `assign.work`（派活）——淘汰闸的管辖对象。
pub const ASSIGN_WORK: &str = "assign_work";

/// 跨主体立约（承诺）——承诺记账的管辖对象。
pub const DELEGATION_PROMISE: &str = "delegation_promise";

/// 写任务表——任务表写侧资格闸与任务声明记账的管辖对象。
pub const TASK_TABLE: &str = "task_table";

/// 宿主认识的**全部**行为名。
///
/// 只用于两件事：装配期/收集期的显式校验，以及判据脚本的读取面。闸门自己按
/// 单个常量判，不遍历这张表——「表里有什么」不是判断。
pub const KNOWN: &[&str] = &[ASSIGN_WORK, DELEGATION_PROMISE, TASK_TABLE];

/// 这个名字是不是宿主已知的行为。
///
/// **未知行为静默不生效**是 J3 要消灭的形态，所以声明面的读取方应当拿它校验；
/// 本函数只回答词汇问题，不替任何闸门决定语义。
pub fn is_known(name: &str) -> bool {
    KNOWN.contains(&name)
}

#[cfg(test)]
#[path = "behaviors.test.rs"]
mod tests;
