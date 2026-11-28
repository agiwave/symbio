//! 收集器的默认实现 —— 收集口 trait 的内存版（`DefaultToolVisitor`）
//!
//! 收集口 trait 与上下文键住在 [`crate::sdk::capability`]；默认实现是全体插件共同
//! 写入的有状态机制，因此归 SDK，而不是嵌入或 VDFS 那类服务实现层。
//! 它只有一种实现 直接组合具体类型，不套 `creator_create_object` 工厂。
//!
//! 一个实现装全部族（工具 / 模型 / 提示词 / 挂载 / 选项 / 可配置）；去重、覆盖与
//! 保序是表语义，收集管线仍归 session / composite。每个收集 pass 新建实例，状态不
//! 跨请求复用。

// 子模块私有：本文件是唯一出口。
mod approval;
mod approval_policy;
mod tool_visitor;

pub use crate::sdk::capability::ApprovalGrants;
pub use crate::sdk::keys::APPROVAL_GRANTS;
pub use approval::{
    approval_gate, approval_gate_with_facts, ApprovalEffect, ApprovalSummary, AsyncFactsOf,
    FactsOf, RiskOf,
};
pub use approval_policy::{
    approval_policy_init_slot, approval_policy_of, approval_policy_publish, ApprovalEvaluation,
    ApprovalPolicy, ApprovalScopes, PolicyBinding, APPROVAL_POLICY,
};
pub use tool_visitor::{
    register_tool, register_tool_with_async_facts, register_tool_with_facts,
    register_tool_with_summary, DefaultToolVisitor,
};

use crate::kernel::capability::Registry;
use std::sync::Arc;

/// 三条收集通道的**共用构造点**（/ 04 报告建议 2）。
///
/// ## 为什么需要一个具名构造点
///
/// `CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIGURABLE_VISITOR` 三个键的值类型
/// 都是 `Arc<dyn Registry>`（见 `crate::sdk::capability::collect`）。类型相同意味着
/// 它们**可以**是同一张表，也意味着装配方可以悄悄挂上三个不同实现——那时「三通道
/// 同表」就裂成三张，没有编译错误，也没有测试会红。
///
/// 把构造收在这里，装配点就只剩「取一个实例、按通道挂键」这一种写法；`DefaultToolVisitor`
/// 因此不会在某个装配点被换成另一种 `Registry`。一次收集过程里若同挂多个键，
/// 挂的必须是**同一个实例的克隆**（`Arc::clone`）——这条由本模块的
/// `one_registry_on_three_keys_is_one_table` 用例钉住。
///
/// ## 为什么仍是**每次收集一个新实例**
///
/// 能力实例可能持有本次请求相关的状态（人格摘要、智能体列表等），跨请求复用会把
/// 上一轮的快照带进下一轮。所以「共用构造点」共用的是**实现与装配写法**，不是
/// 跨请求的单例：每个收集 pass 调一次本函数。
///
/// ## 为什么不提供一个「一次挂三键」的 helper
///
/// 三条通道在**不同遍历**里收集：`session::collect_capabilities`（工具）、
/// `session::collect_options`（选项）、容器的配置声明广播（可配置）。把它们并到
/// 同一次遍历会改变语义——插件在工具遍历里会顺带声明可配置条目
/// （`capability_announce_configurable` 不看路径），而那些行的名字是插件目录名
/// （如 `local`），与同一次广播写入的挂载行同名：同一张表里跨族互相覆盖，
/// 且非工具行混进了工具表。
///
/// 所以这里只收**构造点**，三个键各自挂在哪一次遍历仍归各收集 pass。将来若有
/// 单 pass 真要同挂多键，取一个实例的 `Arc::clone` 分别挂上即可——那条不变量
/// 由 `one_registry_on_three_keys_is_one_table` 钉住。
pub fn new_registry() -> Arc<dyn Registry> {
    Arc::new(DefaultToolVisitor::new())
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
