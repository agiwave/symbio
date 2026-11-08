//! 转发：Token 计量与头尾切分的实现在 `symbio_sdk::text::tokenizer`（`D-404`）。
//!
//! 门面只留**路径**：扩展层与宿主都照旧写 `symbio_plugin_api::text::…`，
//! 但实现住在共享库——纯机制不占门面的行数，也不给扩展插件多一条依赖边。

pub use symbio_sdk::text::tokenizer::*;
