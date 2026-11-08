//! 世界模型插件 —— **世界状态观察 + 读数**（名册 #35，DE-026 §2.3）。
//!
//! 本 crate 是「新增能力不需要改架构」的正向验证之一，与 `symbio-plugin-causal`
//! 用**相同的**扩展面组合，只是折叠口径不同：
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 工具（`Capability`） | `world_observe` | 模型（经工具面调用） |
//! | 行为（`ToolBehavior`） | 一次成功观察 ⇒ 一条 `world.observed` 草稿 | 宿主唯一的写口（I-1） |
//! | 事实（`EventDecl` / `FactDecl` / 归宿行） | `world.observed` 落在 `domain × asserted` | 写闸按声明校验名字与格位 |
//! | 视图（`ViewDecl` + `shared_views`） | `world.model` 折出「每个实体当前各属性」 | 任何按名字读的插件（D-221） |
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
