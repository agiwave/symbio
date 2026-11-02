//! 目标自生成能力包 —— **「欲」的表达与它的升格读数**（名册 #31，DE-026 §2.3 / DE-042）。
//!
//! 本 crate 是「读数类能力搬出控制面」的第一个落点，验证的是 `D-281` 开的那一格：
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 事实（`EventDecl` / `FactDecl` / 归宿行） | `conation.expressed` 落在 `conation × opened` | 写闸按声明校验名字与格位 |
//! | 视图（`ViewDecl` + `shared_views`） | `conation.intents` 折出意图流 + 两个计数 + 采纳率 | 任何按名字读的插件（D-221） |
//! | 多片合并（`ViewDecl::agg`） | N 份单片读数 → 实例级的采纳率 | 实例级聚合列的读方（D-281） |
//!
//! 没有工具、没有路由、没有行为、没有新端口——`kernel` / `sdk` / `loop` / `session` 一行
//! 都不动，`session` 只是把「自己折一遍」换成「按名字读 + 交回合并」。
//!
//! ## 为什么它有 `Plugin` 实现却没有工具与路由
//!
//! 四条扩展面都走链接期收集，实例不参与任何判断；实例存在只为**挂载**——装配清单按插件
//! 目录挂载，停用插件 ⇒ 实例不构造。`route` 恒 `NotFound`、`traverse` 恒空数组，两者都
//! 不是占位符而是答案（与 `symbio-plugin-task` 同一形状、同一理由）。
//!
//! 写入由会话的自主触发完成（会话存储所有者的写窗口），读出由读方按名字求值。
//!
//! ## 依赖面
//!
//! `[dependencies]` 只有 `symbio-plugin-api` 与 `serde_json`。HS-001 判的
//! `kernel` / `sdk` / `deploy` 一个都不列，也不认识任何兄弟插件。

mod autonomy;
pub mod decl;
pub mod plugin;

pub use plugin::{ConationPlugin, PLUGIN_TITLE};
