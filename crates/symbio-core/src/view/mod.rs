//! 视图与预算 —— 封闭语法（`Budget` / `View`）住 `symbio_kernel`，词汇与判据住这里。
//!
//! 智能体命名空间登记表是**词汇**（哪个 namespace 承载本机智能体），由本模块 owner
//! 声明；`Budget` / `View` 是投影框架的形状，由内核提供（DE-023 D-188 C2a）。
//! 既有路径 `crate::view::…` 两类都解析得到。
//!
//! 召回视图（`RecallView` / `RecallEntry`）不在这里：它的产出方与消费方都在
//! `crates/symbio-plugin-loop`，形状归那条投影自己持有（[DE-024](../../../../docs/003-design/024-能力横向扩充的通路.md "DE-024-能力横向扩充的通路") D-201）。

pub use symbio_kernel::view::*;

/// **智能体命名空间登记表**（[D-069 §决策 3](../../../../docs/003-design/005-协议规范.md#d-069-principal-三字段kind-与-id-分家id-是可路由地址 "DE-005-协议规范")）。
///
/// 主体 id 是可路由地址 `<namespace>/<name>`；kind 是元数据，与 id 分家。判定「这条 id
/// 是不是智能体一族」不再靠字符串前缀，而是查这张**登记表**——把 reflex 拆到独立 decider
/// 插件时，登记表加一条 `"decider"` 即可，判据一行不动。
///
/// 今天只有 `session/*` 承载本机智能体；person / tool / system 三类各自住在自己的 namespace 下，
/// 都**不在**这张表里（`visible_to` 的第 3 条「非 agent 一律可见」靠这张表分岔）。
pub const AGENT_NAMESPACES: &[&str] = &["session"];

/// id 的**归属段**（`<namespace>/<name>` 的第一段）——不是主体清单，是路由用的地址解析首段。
pub fn principal_namespace(id: &str) -> Option<&str> {
    id.split_once('/').map(|(ns, _)| ns)
}

/// 这条主体 id 属于**智能体一族**吗？（[D-069 §决策 3]）
///
/// 判据：`namespace` 落在 [`AGENT_NAMESPACES`] 登记表里 ⇒ 智能体；否则 ⇒ 不是。
/// **未识别的 namespace** 也返回 `false`——不是 agent，`visible_to` 第 3 条就会放行；
/// 但授权表 `production_matrix` 会照旧 fail-closed（未知主体 = 无 grants）。
pub fn is_agent_id(id: &str) -> bool {
    principal_namespace(id).is_some_and(|ns| AGENT_NAMESPACES.contains(&ns))
}

/// **可见域的平凡判据**（plan/11 批 1 ③，
/// S08 §4 的 `vis_scope` 平凡值）：`actor` 这条事实在 `viewer` 的视图里可见吗？
///
/// 三条规则，一条判定方（判据在 [] 后由前缀切到命名空间登记表）：
///
/// 1. 自己的发言可见（`actor == viewer`）；
/// 2. 会话内的另一方（人）与**未标身份**的事件可见——会话是一条双向线程，
///    线程里的话彼此都看得见；
/// 3. **别的智能体**的发言不可见——第三方私有，不进本主体的 prompt。
///    「是不是智能体」= [`is_agent_id`]；`person/local` / `vdfs/write` / `system/compactor` 都不是智能体，一律放行。
///
/// 平凡值保证：今天的会话是单主体的（所有智能体发言同一 id `session/main`）⇒ 这条
/// 判定对每一条事件都返回 `true`，视图与接线前**逐字一致**。多主体一出现
/// （子智能体、对等体），第 3 条才开始挡东西——那正是它存在的理由。
pub fn visible_to(actor: &str, viewer: &str) -> bool {
    actor == viewer || !is_agent_id(actor)
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
