//! 会话插件的投影：本插件口径的纯折叠（DE-025 D-221）。
//!
//! 这些投影原先住在 `symbio_core::projection`，但它们的读方全在本插件（`stats` /
//! `changes`）：留在 core 意味着「改一行会话口径要动架构模块」。搬到这里之后
//! core 只保留协议自身的投影（转写 / 轮状态 / 任务词表等），本插件自取自用。

pub(crate) mod changes;
pub(crate) mod checkpoint;
pub(crate) mod fallback;
pub(crate) mod slo;
pub(crate) mod span;
/// 共享读侧（不产投影）：折叠之间要共用的判据放这里，兄弟折叠就不必互导（PP-001）。
pub(crate) mod tiers;
