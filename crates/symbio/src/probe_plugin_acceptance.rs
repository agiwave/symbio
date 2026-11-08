//! 第二个探针（真实能力包）的装配验收：`symbio-plugin-probe` 的 `probe_echo`
//! 端口从 `PLUGIN.yml` 进默认装配，并被生产冻结表接纳。
//!
//! 断言落在 [`tests`](self::tests)：装配清单含 `probe`、端口声明与行都在表上、
//! 选债器按 `request(port=probe_echo)` 选中它。驱动面（选债 → 执行体还事实 → 配对）
//! 由插件自己的 `drive.test.rs` 覆盖。

#[cfg(test)]
#[path = "probe_plugin_acceptance.test.rs"]
mod tests;
