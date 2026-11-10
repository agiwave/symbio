//! 子作用域改名工具的**本名声明面**（D-428）。
//!
//! ## 这一面回答什么
//!
//! 会话装配要知道「哪个前缀工具是根上同名工具的一份副本」。此前它由控制面按
//! **名字形状**猜：试遍前缀里的所有切点，看剩下的基名在不在工具集合里。那份前缀
//! 规则属于子 Agent 插件，控制面不该认识它——前缀一改，猜法就错，而错误是静默的
//! （副本留下 ⇒ 模型看见两个同能力；或独有工具被误删 ⇒ 能力凭空消失）。
//!
//! 本面把答案交给**声明方**：改名包装在 `put` 工具行的同时声明一对
//! `(前缀名, 本名)`；宿主装配只读声明，不解析任何名字、不认识任何前缀形状。
//!
//! ## 为什么复用能力表，而不是新开一条通道
//!
//! 族是值（`CapabilityFamily::multi`），常量与读口住本域，内核零改动；声明沿用
//! 开轮那一次 `traverse` 与 [`CAPABILITY_VISITOR`](super::CAPABILITY_VISITOR)，
//! 不新增上下文键、不新增实体、不新增事件类型。载荷是 `(String, String)`——
//! 原始类型，声明方与读方之间不共享任何结构体，也就没有第二个类型 owner。
//!
//! ## 边界
//!
//! - 行名（表键）必须与工具行不同：表的键是**全表唯一**命名空间（`Registry::put`），
//!   同名的声明行会把工具行顶掉。键的形状由声明方自己给，读侧只用载荷、不解析键。
//! - 声明只答「谁是谁的副本」，**不答该不该删**：本名是否真的在场由装配面的工具集合
//!   判（卸载 / 缺插件时本名缺席，前缀工具必须保留，不能静默丢能力）。
//! - 卸载声明方 ⇒ 没有声明行 ⇒ 一个工具都不删，系统完整运行、不留静默边。

use symbio_kernel::capability::{CapabilityFamily, Registry};

/// 改名工具的本名声明族。**族是值**：新族不需要内核枚举，也不需要改动语法面。
///
/// 名字刻意不用 `Entity` 首段的点分形状（`scope.twin`）：`scope` 是 `Entity::Scope`
/// 的小写，那种串在机制层会被 CS-008 读成事件名字面量，而且「子 Agent 作用域」与
/// 「可见域」本不是一回事。本族是能力表的一项，不是事件族。
pub const SCOPE_TWIN: CapabilityFamily = CapabilityFamily::multi("scope_twin");

/// 宿主侧唯一读口：按注册序折成 `(前缀名, 本名)` 列表。
///
/// 只收载荷形状正确的行；别的族、别的载荷都不进这张表。没有声明时返回空表，
/// 装配照常进行（一个工具都不去重）。
pub async fn capability_scope_twins(visitor: &dyn Registry) -> Vec<(String, String)> {
    visitor
        .list(SCOPE_TWIN)
        .await
        .into_iter()
        .filter_map(|row| row.payload::<(String, String)>().cloned())
        .collect()
}

#[cfg(test)]
#[path = "scoped_twin.test.rs"]
mod tests;
