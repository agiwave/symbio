//! 轮次面的共享门面（D-561）。
//!
//! 造节点与轮次产物记录的实现归 session；`result_node_id` 是内核规则，
//! 保留既有模块路径给 `local` 插件消费。

pub mod node_build;
