//! 时延分层能力包 —— **按派发档折叠的三列读数**（名册 #13，DE-016 §8.2 / DE-008 §3.1）。
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 视图（`ViewDecl` + `shared_views`） | `slo.latency` / `slo.fallback` / `slo.span` 三列按档折叠 | 任何按名字读的插件（`D-221`） |
//! | 多片合并（`ViewDecl::agg`） | N 份单片读数 → 实例级的分位数 / 率 / 中位数 | 实例级聚合列的读方（`D-281`） |
//!
//! 它**不产事实**：三列读的都是别人的格（`chat.assistant.final` 的 `cost_ms` 是信封字段，
//! 档位读 `user.message` 的 `tier`）。所以 `PLUGIN.yml` 里没有 `shared_events`。
//!
//! 没有工具、没有路由、没有行为、没有新端口——`kernel` / `sdk` / `loop` / `session` 一行
//! 都不动，`session` 只是把「自己折一遍 + 自己合一遍」换成「按名字读 + 交回合并」。
//!
//! ## 为什么它有 `Plugin` 实现却没有工具与路由
//!
//! 扩展面都走链接期收集，实例不参与任何判断；实例存在只为**挂载**——装配清单按插件目录
//! 挂载，停用插件 ⇒ 实例不构造。`route` 恒 `NotFound`、`traverse` 恒空数组，两者都不是
//! 占位符而是答案（与 `symbio-plugin-task` 同一形状、同一理由）。
//!
//! ## 依赖面
//!
//! `[dependencies]` 只有 `symbio-plugin-api` 与 `serde_json`。`kernel` / `sdk` / `deploy`
//! 一个都不列，也不认识任何兄弟插件（`E-009`）。

mod decl;
mod plugin;
