//! Classify 插件模块（对话面：判决）
//!
//! 本模块是**私有**的：插件之间互不可见（见 [`crate::plugins`] 的架构原则），
//! 跨插件调用一律经容器 `route` + `symbio_core::{Verdict, DecideRequest}` 的共享契约。
//!
//! ## 域内分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件本体：路由臂 `decide` + 两条产线的顺序 |
//! | [`rules`] | 反射档规则表（**纯函数**，零 LLM 往返） |
//! | [`decide`] | 快速档分类（一次静默 LLM 往返，四选一） |
//! | [`config`] | 本插件自己的配置（`<本插件目录>/PLUGIN.yml`） |
//!
//! 理由码词表**不在本插件里**：它是 `Verdict` 的 `reason` 取值，而消费方在
//! `compose`，故住在 `symbio_core::schemas::dialog`（判据见那里的模块文档）。
//!
//! 内部依赖是单向的：`plugin` → `rules` / `decide`，两者互不认识，也都不认识
//! `config` 之外的任何东西。

mod config;
mod decide;
mod plugin;
mod rules;
