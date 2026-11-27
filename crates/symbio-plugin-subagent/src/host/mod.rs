//! subagent 插件的宿主接入层 —— Agent 目录规范 v2（`agent-dir/v2`）的实现。
//!
//! ## 职责（规范 §3.2 宿主）：只管**装进来的**子智能体
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件主体：装配子智能体插件树（托管）+ manifest 门槛 |
//! | [`store`] | agent 目录存储：**本插件自己的目录**、zip 导入（zip-slip 防护）、导出、条目枚举 |
//! | [`manifest`] | `manifest.yaml` 的读取与接入校验（§5 / §10） |
//! | [`scope`] | `SubAgentVisitor` 代理层：把子树的注册加 `agent/<id>/` 前缀并进系统树（§8.2） |
//! | [`detail`] | agent 概览 / 详情表单的呈现定义 |
//! | [`vdfs`] | VDFS 挂载点（`<根>/subagent/…`，本插件直接 `impl VdfsProvider`） |
//!
//! **本层没有任何自有协议路由**：agent 目录的浏览 / 导入 / 删除 / 导出分别由
//! `vdfs/list`、`vdfs/write`（二进制）、`vdfs/delete`、节点动作 `export` 承担，
//! 原 `agent 目录/*` 协议已下线。
//!
//! ## 本插件是**外部智能体域**的唯一所有者
//!
//! 「拥有装进来的智能体」在这套架构里是两件同源的事，它们必须在同一个插件里：
//!
//! 1. **智能体库**：扫描 / 导入 / 导出 / 删除 agent 目录（[`store`]）；
//! 2. **智能体的装配**：把 agent 目录挂成插件树并收集其能力（[`plugin`] / [`scope`]）。
//!
//! ## 边界：**当前智能体自身**的东西不归本插件
//!
//! 「当前智能体自身」的几样东西各有自己的插件，它们都是**分形**的（系统树与每棵
//! 子树各有一份实例），本插件一律不碰：
//!
//! | 东西 | 归谁 | 地址 |
//! |---|---|---|
//! | 子智能体自身的记忆（`MEMORY.md`） | `memory` | `<根>/memory/MEMORY.md` |
//! | 子智能体自身的指令 / 人格（`AGENTS.md`） | `setting` | `<agentdir>/AGENTS.md`（只读注入段） |
//! | 子智能体自身的信息设置 | `setting` | `<根>/setting/PLUGIN.yml` |
//!
//! 读写面与注入面因此落在同一个所有者上——`memory` 既注入动态记忆又持有其挂载点，
//! 稳定指令 / 人格（`AGENTS.md`）归 `setting` 只读注入。本插件只回答「有哪些智能体、
//! 怎么把它们连进来」。
//!
//! ## 闸门不在本插件
//!
//! 容量闸门随各自能力归属，指令是只读注入；本插件不持有任何闸门，也不解释其它能力的
//! 落位规则。

mod detail;
pub mod manifest;
pub mod plugin;
pub mod scope;
pub mod store;
pub mod vdfs;

#[cfg(test)]
mod tests;
