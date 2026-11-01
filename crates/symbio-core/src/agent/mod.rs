//! Agent 包的权限声明与执行方（`AgentPermissions` / `AgentScope` / `agent_perms_of`）。
//!
//! **实现住 `symbio-sdk`**（D-258）：声明形状与判定只对「这个 agent 声明了哪些能力」说话，
//! 不认容器概念。本模块只转发，`symbio_core::agent::…` 这条稳定路径不变。

// 本域实现已整体搬走（D-258 / D-259），这个文件只剩**登记面**（`core-naming-audit` N-004）：
// 仓内没有消费者，对外也不再经根转发（D-272）。下面的转出因此是「地址」而不是「依赖」。
#![allow(unused_imports)]

pub use symbio_sdk::agent::*;
