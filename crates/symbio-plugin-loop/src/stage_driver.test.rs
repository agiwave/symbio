//! 幕界中止的两条判据（`D-052` 决策 1；`D-074` 让请求计数退役）。
//!
//! 「一轮什么时候收工」在 v2 只有三处：收束格（答复 / 无工具收尾）、幕界中止、
//! 以及装配失败。请求计数**不再是**退出条件（`D-074`）；因此旧用例
//! 「请求计数不作为退出条件」「abort 优先于计数」在这里没有对象可测——计数已不
//! 存在，abort 也没有可与之竞争的第二条判据。留下的是判据本身的形状：
//! **只有幕界**（上一幕已收束）的中止才收工，幕内中止交给该幕自己的收束格记下。

use super::aborts_at_a_stage_boundary;

/// 闲着的轮次不是幕界：没有「已收束的上一幕」，中止判据不成立。
#[test]
fn an_idle_turn_does_not_stop_at_a_boundary() {
    assert!(!aborts_at_a_stage_boundary(None, false));
}

/// 中止只在幕界成立：幕内中止不算幕界出口，且未中止的幕界也不收工。
///
/// `retiring = Some(_)` 是「上一幕已收束、`continue-turn` 还没跑」的唯一标记；
/// 把它拿掉（幕内）或把 `aborted` 拿掉（正常续幕）都不该产生幕界出口。
#[test]
fn an_abort_is_only_a_boundary_event_after_a_stage_closed() {
    assert!(aborts_at_a_stage_boundary(Some(7), true));
    assert!(!aborts_at_a_stage_boundary(None, true));
    assert!(!aborts_at_a_stage_boundary(Some(7), false));
}
