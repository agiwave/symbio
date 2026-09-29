//! Triage 插件模块（对话面：判决）
//!
//! 本模块是**私有**的：插件之间互不可见（见 [`crate::plugins`] 的架构原则），
//! 跨插件调用一律经容器 `route` + `symbio_core::schemas::dialog` 的共享契约。
//!
//! ## 域内分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件本体：路由臂 `decide` + 两条产线的顺序 |
//! | [`rules`] | 反射档规则表（**纯函数**，零 LLM 往返） |
//! | [`classify`] | 快速档分类（一次静默 LLM 往返，四选一） |
//! | [`reasons`] | 理由码词表（`Verdict::reason` 的取值） |
//! | [`config`] | 本插件自己的配置（`<本插件目录>/PLUGIN.yml`） |
//!
//! 四条内部依赖是单向的：`plugin` → `rules` / `classify` → `reasons`。
//! 反过来没有边——`reasons` 不认识任何人，`rules` 与 `classify` 互不认识。

mod classify;
mod config;
mod plugin;
mod reasons;
mod rules;
