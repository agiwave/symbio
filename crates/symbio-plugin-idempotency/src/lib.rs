//! 幂等台账插件 —— **幂等与重试**（名册 #37，DE-026 §2.3）。
//!
//! 这一片只落**幂等**：同一个键在同一会话里只记一次。重试不在本片（它需要重放身份与
//! 续写幕的口径，属后续片），因此本插件的名字与视图都说「台账」，不说「重试」。
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 工具（`Capability`） | `idempotent_record` | 模型（经工具面调用） |
//! | 行为（`ToolBehavior`） | 一次成功记录 ⇒ 一条 `idempotency.recorded` 草稿 | 宿主唯一的写口（I-1） |
//! | 事实（`EventDecl` / `FactDecl` / 归宿行） | `idempotency.recorded` 落在 `domain × asserted` | 写闸按声明校验名字与格位 |
//! | 视图（`ViewDecl` + `shared_views`） | `idempotency.ledger` 折出按键去重的台账 | 任何按名字读的插件（D-221） |
//!
//! 没有新路由、没有配置、没有新端口——`kernel` / `sdk` / `loop` / `session` 一行都不动。
//!
//! ## 依赖面
//!
//! `[dependencies]` 只有 `symbio-plugin-api`（加 `serde_json` / `tokio` / `async-trait`
//! 这些通用库），HS-001 判的 `kernel` / `sdk` / `deploy` 一个都不列。

mod behavior;
mod decl;
mod plugin;
mod tool;

pub(crate) use plugin::{IdempotencyPlugin, PLUGIN_TITLE};
