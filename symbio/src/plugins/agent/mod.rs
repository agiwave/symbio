//! agent —— Agent 目录规范 v2 的宿主插件。
//!
//! ## 定位
//!
//! 规范见 `docs/design/agent-directory-spec.md`（已取代 OAB v1）。核心主张是
//! **父子同构**：系统 Agent 与子 Agent 是同一种东西——都是一棵插件树，由同一个
//! 插件容器装配。
//!
//! 因此这里**没有协议核心模块**：v1 的 `core/`（manifest 数据结构 + 约定目录装配）
//! 已随 v1 一起删除。宿主不为 Agent 单独实现技能 / MCP / 记忆的解释逻辑，一律
//! 复用已有系统（§3.2 第 2 条）——那正是 v1 失败的地方。
//!
//! ## 模块结构
//!
//! - [`host`]：symbio 适配层——manifest 读取与门槛校验、Agent 目录的存储与整包
//!   分发、子 Agent 插件树的托管与注册代理、VDFS 挂载点

pub mod host;
