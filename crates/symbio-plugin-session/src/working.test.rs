//! `working.rs` 单元测试 —— 工作态闭集的迁移判据。
//!
//! 钉的是两件事：状态怎么变、**谁**该被唤醒（伪唤醒不许出现）。

use super::*;

#[test]
fn begin_then_finish_returns_to_idle() {
    let mut state = WorkingState::Idle;

    assert!(
        !state.apply(WorkingTransition::Begin),
        "进入工作态不叫「让出」"
    );
    assert!(state.is_working());

    assert!(
        state.apply(WorkingTransition::Finish),
        "收轮是由忙变闲——等待方必须被唤醒"
    );
    assert!(!state.is_working());
}

#[test]
fn idle_finish_is_a_noop_and_does_not_wake_again() {
    let mut state = WorkingState::Idle;
    assert!(
        !state.apply(WorkingTransition::Finish),
        "空闲时收轮是空操作（一次收轮 = 一次唤醒，不得再叫醒）"
    );
    assert!(!state.is_working());
}

#[test]
fn a_second_begin_after_finish_is_a_real_enter() {
    let mut state = WorkingState::Working;
    assert!(state.apply(WorkingTransition::Finish));
    assert!(!state.apply(WorkingTransition::Begin), "开轮不叫「让出」");
    assert!(state.is_working());
}
