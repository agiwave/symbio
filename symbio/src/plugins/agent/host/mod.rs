//! symbio 宿主适配层 —— OAB 协议（`oab/v1`）的第一个接入方实现。
//!
//! ## 职责（规范 §12 宿主参考实现）
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件主体：`traverse(available_tools)` → 扫描约定目录装配 → 身份工具注册 |
//! | [`handlers`] | 管理路由：`bundle/list\|get\|upload\|export\|delete\|preview` + 统一实体协议 `entities/*` |
//! | [`store`] | bundle 存储：系统目录 `plugins/agent/`、zip 导入（zip-slip 防护）、导出 |
//! | [`capability`] | `agent_identity` 身份工具（提示词片段锚定） |
//! | [`entities`] | 统一实体协议接入（kind=`agent`，前端实体页自动发现） |
//!
//! ## 装配契约（三个能力来源）
//!
//! | 约定目录 | 产出 |
//! |---|---|
//! | `prompts/<name>.md` | 系统提示词片段（frontmatter `priority`，默认 10） |
//! | `skills/<name>/SKILL.md` | 系统提示词片段（priority 默认 50，行业 SKILL 格式） |
//! | `mcps/<name>…` | MCP server 声明 → 宿主 MCP 客户端（**工具唯一来源**） |
//!
//! 协议不定义宿主专有执行器：工具一律经 MCP 接入，宿主复用已有 MCP 机制。
//!
//! ## 协议与宿主的边界
//!
//! 本层只做「约定目录 ↔ 宿主机制」的映射（装配语义 → traverse/collect 能力
//! 收集；bundle 权限 → 声明提示），**不实现协议本身**——协议在
//! [`crate::plugins::agent::core`]，且 core 对本层零依赖。

pub mod capability;
mod detail;
pub mod entities;
pub mod handlers;
pub mod plugin;
pub mod store;
pub mod subagent;

#[cfg(test)]
mod tests;
