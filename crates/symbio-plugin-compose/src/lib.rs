//! Compose 插件模块（对话面：措辞）
//!
//! 本模块是**私有**的：插件之间互不可见（见 `crate::plugins` 的架构原则），
//! 跨插件调用一律经容器 `route` + `symbio_plugin_api::{Verdict, DecideRequest}` 的共享契约。
//!
//! ## 域内分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件本体：路由臂 `compose` + 模板 / 填表的分派 + 配置文档 |
//! | [`debt`] | 生成的行债（`compose-wording-model`，端口 `model.assist`） |
//! | [`templates`] | 模板表（**数据 + 纯函数**，零 LLM 往返） |
//! | [`wording`] | 生成请求的**提示词**（不再有 provider 往返） |
//! | [`config`] | 本插件的旋钮（`PLUGIN.yml`：指令段怎么写） |
//!
//! 理由码词表**不在本插件里**：这些码由 `classify` 产出，本插件只按码选措辞，
//! 所以那份线上词表住在内核的 `dialog` 模块（`crates/symbio-kernel/src/dialog/`，判据见那里的模块文档）。
//!
//! 内部依赖是单向的：`plugin` → `templates` / `wording` / `debt` / `config`。
//! 反过来没有边——`templates` 与 `wording` 互不认识（它们是两条并列产线的两半，
//! 分派在 `plugin`）；`config` 不认识它们中的任何一个
//! （它只管"用户填了什么"，不解释这些值怎么用）。

mod config;
mod debt;
mod plugin;
mod templates;
mod wording;
