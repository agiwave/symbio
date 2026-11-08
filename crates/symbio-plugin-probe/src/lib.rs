//! 验收探针插件（真实 crate，非 `#[cfg(test)]`）。
//!
//! 它钉的是「新增一个**能力包**」的架构成本：建目录 + 写 `PLUGIN.yml` + 代码，
//! `kernel` / `sdk` / `plugin-api` / `loop` / `session` 零改动；装配清单（
//! `symbio-deploy`）、链接行与依赖行都由 `PLUGIN.yml` / 目录生成。
//!
//! 本包**不进默认装配**（没有工具 / 路由 / 挂载点，从 CLI 外面读不到用户可见面）。
//! 它仍被链接并提交声明，由通用判据
//! `crates/symbio/src/assembly_acceptance.test.rs` 覆盖（工厂可构造 + 端口进冻结表
//! 且有受理行）；端口闭环的驱动断言属于 e2e 黑盒，不在宿主 crate 里写专名用例。
//!
//! 本文件**没有 `pub` 项**（E-014）：注册入口是 `plugin.rs` 里的
//! `submit_object_creator!`，不是导出根。

mod declarations;
mod plugin;
mod port;
