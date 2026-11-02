//! 会话插件的投影：本插件口径的纯折叠（DE-025 D-221）。
//!
//! 这些投影原先住在 core 的 `projection`，但它们的读方全在本插件（`stats` /
//! `changes`）：留在 core 意味着「改一行会话口径要动架构模块」。搬到这里之后
//! core 只保留协议自身的投影（转写 / 轮状态 / 任务词表等），本插件自取自用。
//!
//! ## 时延 / 兜底 / 跨度三列已经搬走了
//!
//! 它们（连同「这一轮属于哪一档」这条共用判据）现在住在能力包 `slo`
//! （`crates/symbio-plugin-slo`）：`ViewDecl::eval` 交一片怎么折、`agg` 交 N 片怎么合，
//! 本插件按名字读（`D-221`）、不认识它们的形状（`D-281`）。原来这里的
//! `slo.rs` / `fallback.rs` / `span.rs` / `tiers.rs` 是同一件事的第二份实现——
//! 留着就会长出两套「P99 是什么」。
//!
//! 自主程度（`autonomy`）也搬走了：它与时延族不是同一条口径，但读方形状一样——折叠住在
//! `crates/symbio-plugin-conation`（`conation.autonomy`），本插件按名字读（`D-221` / `D-281`）。

pub(crate) mod changes;
pub(crate) mod checkpoint;
