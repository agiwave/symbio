//! MCP (Model Context Protocol) 插件
//!
//! ## 职责划分（2026-07-06 重构）
//!
//! - **后端**（`symbio/src/plugins/mcp/`）：MCP **配置管理**（CRUD）+ **客户端
//!   transport**（stdio / http，按需 lazy 加载）—— 与系统工具机制集成，
//!   每次 `parent.traverse` 时把已启用的 MCP server 工具注册到 `tool_manager`。
//! - **前端**（`tauri`）：**仅**负责 MCP Server 的配置管理（CRUD UI），不实现
//!   任何 transport 客户端。
//!
//! ## 模块结构
//!
//! - [`plugin`]：MCP 插件入口（`McpPlugin`），实现 traverse（动态注册工具）+ route（CRUD + 持久化）
//! - [`manager`]：`McpManager` —— 无状态 transport 路由器
//! - [`stdio`]：stdio transport 实现（每次调用临时 spawn 子进程）
//! - [`http`]：http transport 实现（每次调用新建短连接）
//! - [`types`]：JSON-RPC 2.0 协议层类型
//! - [`capability`]：把单个 MCP 工具包装为标准 `Capability` 供 agent 工具机制调用
//!
//! ## 存储策略
//!
//! 每个 MCP Server 作为一个独立实体存放在
//! `~/.symbio/plugins/mcps/<name>/server.json`。
//!
//! `McpConfig` 的内存视图（`servers: HashMap<name, McpServerConfig>`）
//! 通过从磁盘加载/回写保持一致。

mod capability;
mod http;
mod manager;
mod plugin;
mod stdio;
pub(crate) mod types;
