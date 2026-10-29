//! Notes 插件 —— **扩展夹具**（DE-025 `EX-001`）
//!
//! 它存在的理由不是「便签功能」，而是把各条扩展面同时走一遍，证明新增能力包只需
//! 依赖 `symbio-plugin-api` 与自己的目录：
//!
//! | 面 | 实现 | 提交点 |
//! |---|---|---|
//! | 工具 | [`tool::NotesTool`] | `Plugin::traverse`（实例被构造才注册） |
//! | 领域事实 | [`decl`] 的 `EventDecl`（`notes.created`，`Domain × Opened`） | `submit_declarations!` |
//! | 上下文段 | [`context::NotesProvider`] | `submit_context_provider!` |
//! | 行为 | [`behavior::NoteCapture`] | `submit_behavior!` |
//! | 视图 | [`decl`] 的 `ViewDecl`（`notes.recent` / `notes.digest` / `notes.insights`） | `submit_declarations!` + `PLUGIN.yml` 的 `shared_views` |
//! | 行执行体 | [`row::NoteIndex`] / [`row::NoteReflect`] | `submit_row!`（通用执行体 `plugin_row`，D-241） |
//! | 模型往返 | [`row::NoteReflect`] 交 `RowStep::Ask`，回执落 `notes.insight` | 宿主代付（`plugin_row` → `round_trip`，D-242） |
//!
//! `core` / `kernel` 不认识「便签」这个词：领域事实落在 `Entity::Domain` 扩展位，
//! 写闸按本插件自己的 `EventDecl` 放行，读侧一律不解释（D-209）。
//!
//! ## 域内分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件本体：工具注册、路由（全部 `NotFound`） |
//! | [`tool`] | `notes_add` 的入参校验与执行 |
//! | [`decl`] | 本插件拥有的那个事件名与格位 |
//! | [`behavior`] | 成功调用 ⇒ 一条 `notes.created` 草稿（写口仍是宿主的） |
//! | [`row`] | 成功的产物 ⇒ `notes.indexed`；已索引的便签 ⇒ 一次模型往返与 `notes.insight` |
//! | [`context`] | 把最近的便签渲染成一段上下文 |
//!
//! 内部依赖是单向的：`plugin` → `tool`；`behavior` / `context` 只读 [`decl`] 的名字，
//! 三者互不认识——它们各自是扩展面的一条实现，装配顺序由宿主按 owner 排。

mod behavior;
mod context;
mod decl;
mod plugin;
mod row;
mod tool;
