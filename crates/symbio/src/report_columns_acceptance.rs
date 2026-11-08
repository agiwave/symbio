//! 全装配下的**缺列判据**（`D-387`）：`session/stats` 顶层报表键集合 == 已登记
//! `slot` 集合，一个不多一个不少。
//!
//! ## 为什么住在这里而不是 session 自己的单测
//!
//! 判据要的是**全部 owner 都链接进来**的 `reports()`。session 的单测进程不链接任何
//! owner（E-009），那里 `reports()` 恒为空，断言会退化成恒真——那正是这条判据要挡的
//! 失效形态（「某个 slot 悄悄没渲染」在空登记上看起来完全正常）。
//!
//! 本 crate 是链接全部插件的装配宿主，且判据只经门面（`symbio_plugin_api::reports`）
//! 读——不引用可卸载的 `session` crate 符号（EX-004 / E-014）。
//!
//! ## 它挡的是什么
//!
//! 动态槽位之后，「有哪几列」只由 owner 的 `submit_reports!` 决定；`SessionStats`
//! 用 `#[serde(flatten)]` 把 [`read_all`](symbio_plugin_api::reports::read_all) 交回的
//! 键抬到出口顶层。若有人把那里改回硬编码列，新增的第 N 条报表登记就会在这里少一个键。
//!
//! 断言的落点在 [`tests`](self::tests)。

#[cfg(test)]
#[path = "report_columns_acceptance.test.rs"]
mod tests;
