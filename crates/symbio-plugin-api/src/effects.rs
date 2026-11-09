//! 效果端口：注册表、扩展点与宏整块住 `symbio_sdk::plugin::effects`（`D-408`）。
//!
//! 门面只留**路径**：插件照旧写 `symbio_plugin_api::effects::…`。`BehaviorDecl` 的采集实现只在
//! 定义类型的 crate 里生成，与 `ToolBehavior` 是同一个类型闭包——拆不开（`D-408`）。
//! `submit_behavior!` 与模块同处转出：插件写 `symbio_plugin_api::submit_behavior!` 不变。
pub use symbio_sdk::plugin::effects::*;
pub use symbio_sdk::submit_behavior;
