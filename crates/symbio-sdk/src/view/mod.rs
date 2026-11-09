//! 视图与预算 —— 封闭语法（`Budget` / `View`）住 `symbio_kernel`，地址解析住这里。
//!
//! `Budget` / `View` 是投影框架的形状，由内核提供（DE-023 D-188 C2a）；本模块只剩
//! 主体 id 的**归属段解析**。既有路径 `crate::view::…` 两类都解析得到。
//!
//! 召回视图（`RecallView` / `RecallEntry`）不在这里：它的产出方与消费方都在
//! `crates/symbio-plugin-loop`，形状归那条投影自己持有（[DE-024](../../../../docs/003-design/024-能力横向扩充的通路.md "DE-024-能力横向扩充的通路") D-201）。

// `Budget` / `View` 的数据面在 `symbio_kernel`，本模块只做同名转出（登记面，N-004）；
// 判据（`principal_namespace`）是 core 的。
#[allow(unused_imports)]
pub use symbio_kernel::view::*;

/// id 的**归属段**（`<namespace>/<name>` 的第一段）——不是主体清单，是路由用的地址解析首段。
///
/// 「什么地址算哪一族」只在这里回答一次：主体派生（`authz::principal_of` 认下已带
/// 命名空间的地址）与测试面的地址形状判据都读它。
pub fn principal_namespace(id: &str) -> Option<&str> {
    id.split_once('/').map(|(ns, _)| ns)
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
