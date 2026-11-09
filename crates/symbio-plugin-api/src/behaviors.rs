//! 行为名的收集面：实现住 `symbio_sdk::plugin::behaviors`（`D-408`）。
//!
//! 门面只留**路径**：插件照旧写 `symbio_plugin_api::behaviors::…`。收集面按
//! `inventory` 的注册表折出「名字 → 造格声明」，与 `effects` 是同一个类型闭包，
//! 因此随它整块搬进机制臂（`D-408`）。

pub use symbio_sdk::plugin::behaviors::*;
