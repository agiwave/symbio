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
//! | [`classify`] | 快速档分类（一次静默 LLM 往返，四选一） |
//! | [`reasons`] | 理由码词表（`Verdict::reason` 的取值） |
//! | [`config`] | 本插件自己的配置（`<本插件目录>/PLUGIN.yml`） |
//!
//! 四条内部依赖是单向的：`plugin` → `rules` / `classify` → `reasons`。
//! 反过来没有边——`reasons` 不认识任何人，`rules` 与 `classify` 互不认识。

mod config;
mod decide;
mod plugin;
/// **唯一**对仓内可见的域内模块，理由只有一条：`compose` 要拿它比对措辞表的抄本
/// （`compose/templates.test.rs` 的抄本一致性用例）。
///
/// 它**不放宽插件隔离**：生产代码里跨插件 `use crate::plugins::<兄弟>` 仍被
/// `plugin-entry-audit` 的 E-009 拦着，本条只让**测试**能比对词表。词表是跨插件的
/// 线上词汇表，而它的两份抄本一旦漂移，表现是"用户收到一句通用话"——没有错误信号。
/// 那正是必须外置成断言的那类失效。
pub(crate) mod reasons;
mod rules;
