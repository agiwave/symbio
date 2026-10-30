//! Agent 包的权限声明与执行方（`AgentPermissions` / `AgentScope` / `agent_perms_of`）。
//!
//! **实现住 `symbio-sdk`**（D-258）：声明形状与判定只对「这个 agent 声明了哪些能力」说话，
//! 不认容器概念。本模块只转发，`symbio_core::agent::…` 这条稳定路径不变。

pub use symbio_sdk::agent::*;
