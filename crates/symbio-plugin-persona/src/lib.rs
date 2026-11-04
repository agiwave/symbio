//! 人格插件 —— **人格语气一致**（名册 #47，DE-026 §2.3）。
//!
//! 本 crate 是「新增能力不需要改架构」的一次**正向验证**：它只往已有的提示词族里
//! 放一行（`CapabilityRow::prompt`），`kernel` / `sdk` / `loop` / `session` 一行都
//! 没动。机制与取舍见 [`plugin`] 的模块文档，配置面见 [`config`]。
//!
//! ## 它占用的扩展面
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 提示词族（`family::PROMPT`） | 一条段 `persona-tone` | `session`（轮内正文的系统提示词）与 `compose`（措辞请求的系统提示词），二者都经 `capability_prompts` 读 |
//! | 配置（`PluginConfigFile`） | `<根>/persona/PLUGIN.yml` 的 `persona` 键 | 设置页（`capability_announce_configurable`）与 VDFS（`PluginConfigMount` 四臂） |
//!
//! 没有工具、没有新的事实、没有新的路由、没有新端口——**这正是本条要证明的事**：
//! 「同一个人在两条输出面上口吻一致」不需要宿主为它加一格影响类。
//!
//! ## 依赖面
//!
//! `[dependencies]` 只有 `symbio-plugin-api` 一行（HS-001）：`kernel` / `sdk` /
//! `deploy` 一个都不列，插件能看见什么由那道门面逐条决定。

mod config;
mod plugin;

pub(crate) use config::PersonaConfig;
pub(crate) use plugin::{PersonaPlugin, PLUGIN_TITLE, SEGMENT_NAME};
