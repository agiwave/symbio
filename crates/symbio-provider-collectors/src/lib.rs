//! 收集器的默认实现 —— 收集口 trait 的内存版（`DefaultToolVisitor`）
//!
//! ## 为什么在 `providers/` 而不是 `symbio_core`
//!
//! 收集口 **trait** 是契约（住 `symbio_core::capability`）；它的**默认实现**
//! 是「不存在第二种实现的机制底座」，因此按 `crate::providers` 的第二种接线方式
//! **直接组合具体类型，不套 `dyn` 工厂**——再套一层 `creator_create_object` 只是把一次
//! 构造换成一次字符串查表（与 `vdfs_service` 同款判断；D-035 决策 1.1「≥2 实现才
//! provider 化」因此**不适用于这里**）。
//!
//! 4e 之后**一个实现装全部族**（工具 / 模型 / 提示词 / 挂载 / 选项 / 可配置）：
//! 此前选项与可配置声明各有自己的 `Default*Visitor`（各 2 方法的专服单族 trait），
//! 随两个 trait 一并退场——去重、覆盖、排序（选项的 `order` 在读侧）都是表或读协议
//! 的语义，不再各住一份实现。
//!
//! ## 为什么不是「归安装它的那个插件」
//!
//! 收集器的**写入者是全体插件**（经 `ctx` 的 `CAPABILITY_VISITOR` / `OPTION_VISITOR` /
//! `CONFIGURABLE_VISITOR` 键），安装它的宿主只是**其中之一**——它是跨插件的共享实现，
//! 不是任何插件的内部物。
//!
//! 实测证据：`vdfs` / `agent` / `work` 三个插件的测试都需要一个能读回的收集器。若它住
//! `session`，这三个测试就要跨插件引用 session 的类型，违反「插件之间不直接相互引用」。
//! 住本层则任何模块（含各插件测试）都能用，**不构成插件间依赖**。
//!
//! ## 契约与实现的分工
//!
//! | 部分 | 住哪 |
//! |---|---|
//! | 收集口（`Registry` 三方法）· 上下文键（`*_VISITOR`）· 遍历端点字面量 | `symbio_core::capability` |
//! | 收集管线（`collect_capabilities` / `collect_options`） | 各自的宿主（session / composite） |
//! | **内存版收集器**（本目录） | `providers/` |
//!
//! 只有「无策略的内存实现」放这里；排序 / 去重之外的任何策略都归调用方。

// ⭐ 子模块一律私有（与 `providers/embedding` 同一约定）：**本文件是唯一出口**。
mod approval;
mod tool_visitor;

pub use approval::approval_gate;
pub use tool_visitor::DefaultToolVisitor;

use std::sync::Arc;
use symbio_core::Registry;

/// 三条收集通道的**共用构造点**（/ 04 报告建议 2）。
///
/// ## 为什么需要一个具名构造点
///
/// `CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIGURABLE_VISITOR` 三个键的值类型
/// 都是 `Arc<dyn Registry>`（见 `symbio_core::capability::collect`）。类型相同意味着
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
#[path = "lib.test.rs"]
mod tests;
