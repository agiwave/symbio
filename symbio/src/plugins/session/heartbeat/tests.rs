//! `heartbeat` 模块的单元测试。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
//! `heartbeat.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// 空闲基线取较新者：正常路径 updated_at 推进到回合结束（活动真正结束），
/// 锚点较旧不拖慢触发；异常路径（零写盘退出）锚点较新，防止逐 tick 热循环。
#[test]
fn idle_baseline_takes_newer_of_anchor_and_updated_at() {
    // 无内存锚点（进程重启后）：回退到磁盘侧 updated_at
    assert_eq!(idle_baseline(None, 1_000), 1_000);
    // 正常路径：updated_at（回合最后一次写盘）较新 → 取 updated_at
    assert_eq!(idle_baseline(Some(2_000), 5_000), 5_000);
    // 异常路径：锚点（触发时刻）较新 → 取锚点（防热循环下限）
    assert_eq!(idle_baseline(Some(8_000), 5_000), 8_000);
    // 相等时取任一即可
    assert_eq!(idle_baseline(Some(5_000), 5_000), 5_000);
}
