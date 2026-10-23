//! `chat_loop.rs`（主循环骨架）的单元测试 —— 闸门 [`gate_turn`] 的契约测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着**被测试的实现文件**走——
//! `state.rs` 的测试在 `chat_loop/state.test.rs`，`inputs.rs` 的在
//! `chat_loop/inputs.test.rs`；`turn.rs` / `compress.rs` / `io.rs` 暂无测试。
//!
//! `gate_turn` 是主循环顶部的**唯一判定点**（退出条件）。收口前这些判定散落在
//! 主循环体里，只能靠端到端跑会话间接验证；收口成纯函数后可以在这里直接钉住语义。
//!
//! 闸门现在只有一条退出条件（中止边界）：「等工具齐」随 v1 外壳的簿记坍缩
//! （[D-073](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")），「工具轮软上限」随它恒 0 的计数一起退役（[D-074](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")）——
//! 工具轮数在这一层不可见，本文件因此**不该**再出现任何按轮次的判定。

use super::*;

fn state(request_round: usize, aborted: bool) -> TurnState {
    let abort = ExecAbortSignal::new();
    if aborted {
        abort.abort();
    }
    TurnState {
        abort,
        request_round,
        ..Default::default()
    }
}

#[test]
fn idle_state_proceeds() {
    assert!(matches!(gate_turn(&state(0, false)), Gate::Proceed));
}

#[test]
fn request_count_is_not_an_exit_condition() {
    // 续写轮（请求数 >0）照样放行：这一层没有轮次上限可判（真实轮数在运行器环体里）。
    assert!(matches!(gate_turn(&state(9999, false)), Gate::Proceed));
}

#[test]
fn abort_at_loop_boundary_exits_without_bubbling() {
    // 边界检查点用 `AbortedAtBoundary`（收尾为 Ok），不是 `Aborted`（收尾时冒泡
    // 中止出口）：上一轮 Turn 已定稿落库，冒泡会让消费循环的
    // `persist_failure` 把已成功的 Turn 误标为 `Aborted`。
    assert!(matches!(
        gate_turn(&state(1, true)),
        Gate::Exit(TurnExit::AbortedAtBoundary)
    ));
}

#[test]
fn abort_wins_over_request_count() {
    // 中止是唯一的退出条件：轮次再多也不构成另一条（这一层读不到工具轮数，见 D-074）。
    assert!(matches!(
        gate_turn(&state(2, true)),
        Gate::Exit(TurnExit::AbortedAtBoundary)
    ));
}
