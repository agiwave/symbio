//! 时钟工具 —— 全项目「当前时间（Unix 毫秒）」的唯一实现。
//!
//! ## 为什么需要
//!
//! 同一个语义（当前时间 → Unix 毫秒 `i64`）此前在三个层里**各写了一份**，共 5 处：
//!
//! | 位置 | 写法 |
//! |---|---|
//! | `symbio_core/turn.rs`（2 处内联） | `time::OffsetDateTime` |
//! | `plugins/session/heartbeat.rs` | `time::OffsetDateTime` |
//! | `plugins/session/plugin.rs` | `SystemTime` + `unwrap_or(0)` |
//! | `providers/vdfs_service/memory.rs` | `SystemTime` + `unwrap_or(0)` |
//!
//! 两种写法对本项目的时间范围等价，但**分散在层间**意味着改口径时必然漏改。
//! 收敛到本模块后全项目共用一份。
//!
//! 口径：**Unix 纪元起的毫秒数**（UTC，`i64`）。
//!
//! ⚠️ 模块名取 `clock` 而非 `time`：后者会与外部 crate `time` 同名，使 crate 内
//! `time::OffsetDateTime` 的解析产生歧义（同 `plugin.rs` 不能用 `mod vdfs` 的陷阱）。

/// 当前时间的 Unix 毫秒时间戳（UTC）。
pub fn clock_now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}
