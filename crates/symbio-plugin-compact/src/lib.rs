//! 上下文压缩能力包 —— **横向扩展**：不认识 session，也不改环。
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | `port` 族行（`<port:compact>:compact`） | 执行体：读本轮终答的水位，超阈值时压缩 | 环按端口名解析提供者 |
//! | `debt` 族行（`<debt>compact`） | 单行债 `compact-run`：有本轮终答且未压缩就欠 | 环按行取债 |
//!
//! ## 它落的那条事实
//!
//! `session.consolidated`（`message × progressed`）——**已有种类**，声明 owner 是
//! `crates/symbio-plugin-session/src/declarations.rs:27`。重建面把它当「整段被摘要
//! 取代」折叠（`symbio_sdk::projection::rebuild`），所以压缩节点在界面上是折叠叶
//! （`NodeKind::Notice` + `style_tag = compression`），读侧零改动。
//!
//! **已知项**：声明的 owner 与写方分离（session 声明、compact 写）。是否要求写方
//! 也声明同一格，等 E-009 / CS-007 裁。
//!
//! ## 它不做什么
//!
//! - 不改 `session.config`（阈值住本 crate）；
//! - 不往 `plugin-api` 加契约或常量（种类名写字面量，注释指向声明点）；
//! - 未超阈值**不落事实**：那是「没做」，不是一条领域事实——执行体照常返回成功，
//!   环照常还债。
//!
//! 依赖面只有 `symbio-plugin-api` / `serde_json` / `async-trait`，不认识任何兄弟插件。

mod debt;
mod plugin;
mod port;
