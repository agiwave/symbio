//! OAB 声明式数据结构与约定目录装配（零宿主依赖）。
//!
//! - [`manifest`]：manifest.yaml 数据结构（§4）；
//! - [`validate`]：加载期校验（接入判定的第一道门，§11）；
//! - [`assembly`]：约定目录扫描与装配（§3 / §5），产出提示词片段 + MCP server 声明。
//!
//! ## 三个能力来源（存在即安装）
//!
//! | 约定目录 | 协议语义 |
//! |---|---|
//! | `prompts/<name>.md` | OAB 原生：系统提示词片段（frontmatter 可带 `priority`） |
//! | `skills/<name>/SKILL.md` | 行业 Skill：正文作为提示词片段（priority 50） |
//! | `mcps/<name>…` | 行业 MCP：server 配置（**工具唯一来源**） |
//!
//! manifest 不重复登记目录里已经表达的事实。
//!
//! 目录名统一为复数（与行业习惯一致）：`prompts/` `skills/` `mcps/`。
//!
//! 工具**只**来自 MCP——协议不定义任何宿主专有执行器（如早期草案的 `oab.echo`），
//! 否则每个接入方都要实现它，协议就失去了通用性。

pub mod assembly;
pub mod manifest;
pub mod validate;
