//! Glossary 插件 —— 项目术语表。
//!
//! 能力面（全部只依赖 `symbio-plugin-api` 与自己的目录）：
//!
//! | 面 | 实现 | 提交点 |
//! |---|---|---|
//! | 工具 | [`tool::GlossaryTool`]（`glossary_define`） | `Plugin::traverse`（实例被构造才注册） |
//! | 领域事实 | [`decl`] 的 `glossary.defined`（`Domain × Opened`） | `submit_declarations!` |
//! | 效果 | [`behavior::GlossaryRecord`] | `submit_behavior!` |
//! | 前置判断 | [`behavior::GlossaryRecord::before`]（同样的释义重复定义在执行前拒） | 同一个 `submit_behavior!` |
//! | 上下文段 | [`context::GlossaryProvider`]（`glossary_context`） | `submit_context_provider!` |
//!
//! 读面是**上下文注入**：每轮把当前术语表（同一术语取最新释义，至多 20 条）拼进
//! 提示词。视图默认私有，本插件不导出跨插件视图。
//!
//! ## 域内分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件本体：工具注册、路由（全部 `NotFound`） |
//! | [`tool`] | `glossary_define` 的入参校验与执行 |
//! | [`decl`] | 本插件拥有的名字、格位与归宿行 |
//! | [`behavior`] | 成功调用 ⇒ 一条 `glossary.defined` 草稿；重复释义在执行前拒 |
//! | [`context`] | 把当前术语表渲染成一段上下文 |
//!
//! 内部依赖是单向的：`plugin` → `tool`；`behavior` / `context` 只读 [`decl`] 的名字。

mod behavior;
mod context;
mod decl;
mod plugin;
mod tool;
