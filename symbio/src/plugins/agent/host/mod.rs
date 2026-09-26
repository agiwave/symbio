//! agent 插件的宿主接入层 —— Agent 目录规范 v2（`agent-dir/v2`）的实现。
//!
//! ## 职责（规范 §3.2 宿主）
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件主体：`traverse` 里的托管（装配子 Agent 插件树）+ 门槛（manifest 校验）+ 指令注入 |
//! | [`store`] | agent 目录存储：**本插件自己的目录**、zip 导入（zip-slip 防护）、导出、条目枚举 |
//! | [`memory`] | **子智能体**自身的 `AGENTS.md`（`<agentdir>/AGENTS.md`）：落位、地址、注入 |
//! | [`instruction`] | **系统智能体**自身的 `AGENTS.md`（`{homedir}/AGENTS.md`）：落位、地址、注入 |
//! | [`config`] | 插件配置（智能体自身 `AGENTS.md` 的写入与注入两道闸门，见 §「闸门」） |
//! | [`manifest`] | `manifest.yaml` 的读取与接入校验（§5 / §10） |
//! | [`scope`] | `SubAgentVisitor` 代理层：把子树的注册加 `agent/<id>/` 前缀并进系统树（§8.2） |
//! | [`subagent`] | `agent_run`（子智能体委托）能力 |
//! | [`detail`] | agent 概览 / 详情表单的呈现定义 |
//! | [`vdfs`] | VDFS 挂载点（`<根>/agent/…`，本插件直接 `impl VdfsProvider`） |
//!
//! **本层没有任何自有协议路由**：agent 目录的浏览 / 导入 / 删除 / 导出分别由
//! `vdfs/list`、`vdfs/write`（二进制）、`vdfs/delete`、节点动作 `export` 承担，
//! 原 `agent 目录/*` 协议已下线。
//!
//! ## 本插件是智能体域的**唯一所有者**
//!
//! 「拥有智能体」在这套架构里包含三件同源的事，它们必须在同一个插件里：
//!
//! 1. **智能体库**：扫描 / 导入 / 导出 / 删除 agent 目录（[`store`]）；
//! 2. **智能体的装配**：把 agent 目录挂成插件树并收集其能力（[`plugin`] / [`scope`]）；
//! 3. **智能体自身的指令**：`{homedir}/AGENTS.md` 与 `<agentdir>/AGENTS.md`
//!    （[`instruction`] / [`memory`]）。
//!
//! 第 3 件事曾经散落在别处（`session` 读系统那一份，子树的 `work` 实例读 agent 目录那一份），
//! 也一度试图收进 `plugin_manager`——但 `plugin_manager` 是**设置页的入口**（自有分区 + 各插件配置清单），
//! 不是任何内容文件的所有者。指令属于智能体域，于是回到本插件：
//! **读写面与注入面落在同一个所有者上**。
//!
//! ## 闸门
//!
//! Agent 目录里所有写入都由本插件执行，因此闸门取值只有一个来源（[`config`]）：
//!
//! - `memory_max_bytes` / `memory_inject_max_bytes`：智能体自身的 `AGENTS.md`
//!   的写入与注入上限——**两个作用域共用一对**（它们是同一类东西，只是作用域不同）。
//!
//! 闸门的**执行**全在共享实现（`providers/memory`），与 work / session 几层同源。

mod config;
mod detail;
pub mod instruction;
pub mod manifest;
pub mod memory;
pub mod plugin;
pub mod scope;
pub mod store;
pub mod subagent;
pub mod vdfs;

#[cfg(test)]
mod tests;
