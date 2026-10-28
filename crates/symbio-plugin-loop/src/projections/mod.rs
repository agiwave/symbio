//! 义务环插件的投影（DE-025 D-221）。
//!
//! 巩固接受边界原先住在 `symbio_core::projection::consolidate`，唯一读方是本插件的
//! `memory_events`——判定口径属于调用方，不属于架构。搬出后 core 不再出现
//! 「巩固」这个词。

pub(crate) mod consolidate;
