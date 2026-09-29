//! actor 插件 —— **可选**的 Actor 登记方与内省入口。
//!
//! ## 它做什么
//!
//! 1. **登记** §2.3 的"抢占判定者"一行（`session.decider`，`budget_ms = 80`）
//!    ——这是"新 Actor 零改 `chat_loop` 接入"的**演示**：本插件只调
//!    [`ActorSource::register_all`]，不碰主循环一个字；
//! 2. 提供 `actor/list` 路由，把登记表暴露给审计者 / 测试 / 调试者
//!    （否则"表在跑"与"表是空的"在系统外部无法区分）。
//!
//! ## 为什么它必须可选
//!
//! 它**不在** [`ASSEMBLY_SUB_AGENT_PLUGINS`](crate::symbio_core::ASSEMBLY_SUB_AGENT_PLUGINS)
//! 里：停用它 ⇒ 判定者行不登记 ⇒ `actor/list` 只剩 session 侧那一行 ⇒
//! **会话照常运行**（J2）。这与 `fact_log` / `projection` 同款——
//! "机制在 core、能力在插件"，能力缺席不改变系统能否运转。
//!
//! ## 它不认识谁
//!
//! - 不认识 `session`：登记只经 core 的 [`ActorSpec`] / [`ActorSource`]，
//!   行名是字符串常量（`crate::symbio_core::ACTOR_NAME_SESSION_REASONER`
//!   的**字面量副本**见下），不 `use` session 的任何符号；
//! - 不认识 `projection` / `fact_log`：三者互不知情，各自的缺席互不影响。
//!
//! 见 `symbio/src/plugins/mod.rs` 的插件独立原则。

mod plugin;
