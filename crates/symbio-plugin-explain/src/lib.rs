//! 可解释插件 —— **可解释**（名册 #49，DE-026 §2.3）。
//!
//! 本 crate 是「新增能力不需要改架构」的正向验证之一，与 `symbio-plugin-causal`
//! 用**同一组**扩展面：
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 工具（`Capability`） | `explain_record` | 模型（经工具面调用） |
//! | 行为（`ToolBehavior`） | 一次成功记录 ⇒ 一条 `explain.rationale` 草稿 | 宿主唯一的写口（I-1） |
//! | 事实（`EventDecl` / `FactDecl` / 归宿行） | `explain.rationale` 落在 `domain × asserted` | 写闸按声明校验名字与格位 |
//! | 视图（`ViewDecl`） | `explain.rationales` 折出理由台账 | 本包私有：没有第二读者，名字不进 `shared_views`（D-221 的公开动作只对真有读方的视图做） |
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
