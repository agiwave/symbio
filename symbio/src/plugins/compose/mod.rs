//! Compose 插件模块（对话面：措辞）
//!
//! 本模块是**私有**的：插件之间互不可见（见 [`crate::plugins`] 的架构原则），
//! 跨插件调用一律经容器 `route` + `symbio_core::{Verdict, DecideRequest}` 的共享契约。
//!
//! ## 域内分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件本体：路由臂 `compose` + 两条产线的分派 + 配置文档 |
//! | [`templates`] | 模板表（**数据 + 纯函数**，零 LLM 往返） |
//! | [`wording`] | 答话生成（一次静默 LLM 往返，仅 `from_context` 走） |
//! | [`config`] | 本插件的旋钮（`PLUGIN.yml`：用哪个模型 / 指令段怎么写） |
//!
//! 理由码词表**不在本插件里**：这些码由 `classify` 产出，本插件只按码选措辞，
//! 所以那份线上词表住在 `symbio_core::schemas::dialog`（判据见那里的模块文档）。
//!
//! 内部依赖是单向的：`plugin` → `templates` / `wording` / `config`。
//! 反过来没有边——`templates` 与 `wording` 互不认识（它们是两条并列的产线，
//! 分派在 `plugin`）；`config` 不认识它们中的任何一个
//! （它只管"用户填了什么"，不解释这些值怎么用）。

mod config;
mod plugin;
mod templates;
mod wording;
