//! Classify 插件模块（对话面：判决）
//!
//! 本模块是**私有**的：插件之间互不可见（见 `crate::plugins` 的架构原则），
//! 跨插件调用一律经容器 `route` + `symbio_plugin_api::{Verdict, DecideRequest}` 的共享契约。
//!
//! ## 域内分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件本体：路由臂 `decide` + 两条产线的顺序 |
//! | [`rules`] | 反射档规则表（**纯函数**，零 LLM 往返） |
//! | [`decide`] | 快速档分类（一次静默 LLM 往返，四选一） |
//! | [`config`] | 本插件自己的配置（`<本插件目录>/PLUGIN.yml`） |
//! | [`decl`] | 事件名 / 事实形状 / 事实归宿行（装载期声明） |
//! | [`debt`] | 债表：这一轮欠不欠一次判决（挂载后生效，带运行期开关） |
//! | [`row`] | 行执行体：规则命中 ⇒ 一条判决事实（0 次模型往返） |
//!
//! 理由码词表**不在本插件里**：它是 `Verdict` 的 `reason` 取值，而消费方在
//! `compose`，故住在内核的 `dialog` 模块（`crates/symbio-kernel/src/dialog/`，判据见那里的模块文档）。
//!
//! 内部依赖是单向的：`plugin` → `rules` / `decide` / `debt`，`debt` → `rules`，
//! `row` → `decl`，各行互不认识。

mod config;
mod debt;
mod decide;
mod decl;
mod plugin;
mod row;
mod rules;
