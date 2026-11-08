//! 验收探针插件（真实 crate，非 `#[cfg(test)]`）。
//!
//! 它钉的是「新增一个**能力包**」的架构成本：建目录 + 写 `PLUGIN.yml` + 代码，
//! `kernel` / `sdk` / `plugin-api` / `loop` / `session` 零改动；装配清单（
//! `symbio-deploy`）、链接行与依赖行都由 `PLUGIN.yml` / 目录生成。
//!
//! 判据见同 crate 的 `drive.test.rs`（环选债 → 执行体还事实）与
//! `crates/symbio/src/probe_plugin_acceptance.test.rs`（默认装配 / 冻结表接纳）。
//!
//! 本文件**没有 `pub` 项**（E-014）：注册入口是 `plugin.rs` 里的
//! `submit_object_creator!`，不是导出根。

mod declarations;
mod plugin;
mod port;
