//! `working.rs` 单元测试 —— 工作态闭集的迁移判据。
//!
//! 钉的是三件事：状态怎么变、**谁**该被唤醒、伪唤醒不许出现。

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
fn early_exit_wakes_once_and_the_late_finish_does_not_wake_again() {
    let mut state = WorkingState::Working;

    assert!(
        state.apply(WorkingTransition::EarlyExit),
        "提前让位同样是「由忙变闲」"
    );
    assert!(
        !state.apply(WorkingTransition::Finish),
        "收尾完成时已经空闲：不得再唤醒一次（一次收轮 = 一次唤醒）"
    );
    assert!(!state.is_working());
}

#[test]
fn idle_paths_never_report_a_handover() {
    let mut state = WorkingState::Idle;
    assert!(
        !state.apply(WorkingTransition::EarlyExit),
        "空闲时让位是空操作"
    );
    assert!(
        !state.apply(WorkingTransition::Finish),
        "空闲时收轮是空操作"
    );
    assert!(!state.is_working());
}

#[test]
fn a_second_begin_after_early_exit_is_a_real_enter() {
    // 提前让位之后、收尾写完之前，直连路径可能开新一轮（`entry` 的兜底串行化）。
    // 这是**真的**进入工作态，而不是上一次的残留。
    let mut state = WorkingState::Working;
    assert!(state.apply(WorkingTransition::EarlyExit));
    assert!(!state.apply(WorkingTransition::Begin), "开轮不叫「让出」");
    assert!(state.is_working());
}
