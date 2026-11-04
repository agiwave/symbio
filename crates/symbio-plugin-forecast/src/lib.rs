//! 未来预测插件 —— **名册 #34「未来预测」的第一条读数型能力**（DE-026 §2.3）。
//!
//! 本 crate 是 `D-294` 首条准入能力的落地：只用**读侧折叠**这一格，不新增事实、
//! 不新增工具、不新增端口，`kernel` / `sdk` / `loop` / `session` 一行都不动。
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 视图（`ViewDecl` + `shared_views`） | `forecast.remaining` 折 `task.opened` / `task.asserted` | 任何按名字读的插件（`session` 的 `capability_views`，D-221） |
//!
//! 账行里的**移除计划**（`DE-026 §2.3` #34）写明期限、触发条件与承接任务：这一格
//! 只由一条能力撑着，所以它带撤除路径（`D-294`）。
//!
//! ## 依赖面
//!
//! `[dependencies]` 只有 `symbio-plugin-api`（加 `serde_json` / `async-trait` /
//! `tracing` 这些通用库），HS-001 判的 `kernel` / `sdk` / `deploy` 一个都不列。

mod decl;
mod plugin;

pub(crate) use plugin::{ForecastPlugin, PLUGIN_TITLE};
