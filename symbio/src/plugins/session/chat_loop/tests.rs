//! `chat_loop.rs`（主循环骨架）的单元测试 —— 闸门 [`gate_turn`] 的契约测试。
//!
//! 与实现分文件（约定同 `store/tests.rs`）：测试跟着**被测试的实现文件**走——
//! `state.rs` 的测试在 `chat_loop/state/tests.rs`，`inputs.rs` 的在
//! `chat_loop/inputs/tests.rs`；`turn.rs` / `compress.rs` / `io.rs` 暂无测试。
//!
//! `gate_turn` 是批次 A 引入的**唯一判定点**（启动条件 / 退出条件）。收口前这些
//! 判定散落在主循环体里，只能靠端到端跑会话间接验证；收口成纯函数后可以在这里
//! 直接钉住语义——尤其是它的**判定顺序**（启动条件优先于所有退出条件），它决定了
//! 异步工具调用（批次 D）会不会被误判为"可以唤醒"。

use super::*;

fn req(max_tool_rounds: Option<usize>) -> TurnRequest {
    TurnRequest::new(&model_chat::Request {
        max_tool_rounds,
        ..Default::default()
    })
}

fn state(tool_rounds: usize, in_flight: &[&str], aborted: bool) -> TurnState {
    TurnState {
        abort_flag: Arc::new(AtomicBool::new(aborted)),
        tool_rounds,
        in_flight_tools: in_flight.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

// ── 启动条件（异步工具调用的唤醒语义）──────────────────────────────────

#[test]
fn idle_state_proceeds() {
    assert!(matches!(
        gate_turn(&req(None), &state(0, &[], false)),
        Gate::Proceed
    ));
}

#[test]
fn in_flight_tools_block_wakeup() {
    // 「多个工具调用全部结束后才唤醒，不完整不唤醒」——剩一个未完成即不唤醒。
    assert!(matches!(
        gate_turn(&req(None), &state(1, &["tc1"], false)),
        Gate::WaitForTools
    ));
}

#[test]
fn wait_for_tools_takes_priority_over_exit_conditions() {
    // 判定顺序契约：在途工具非空 ⇒ 即使已达软上限、且已 abort，仍先返回
    // WaitForTools。批次 D 若把顺序写反，会在工具还没跑完时以 MaxToolRounds
    // 提前退出，把已完成的结果丢掉。
    let turn = state(5, &["tc1", "tc2"], true);
    assert!(matches!(
        gate_turn(&req(Some(5)), &turn),
        Gate::WaitForTools
    ));
}

// ── 退出条件 ①：显式软上限 ─────────────────────────────────────────────

#[test]
fn max_tool_rounds_reached_exits_with_reason() {
    match gate_turn(&req(Some(3)), &state(3, &[], false)) {
        Gate::Exit(TurnExit::MaxToolRounds { max }) => assert_eq!(max, 3),
        other => panic!("期望 MaxToolRounds {{ max: 3 }}，实得 {other:?}"),
    }
}

#[test]
fn below_max_tool_rounds_proceeds() {
    assert!(matches!(
        gate_turn(&req(Some(3)), &state(2, &[], false)),
        Gate::Proceed
    ));
}

#[test]
fn none_means_unbounded_rounds() {
    // 默认不设硬上限（智能体会话轮次越来越多）：None ⇒ 轮次再多也放行。
    assert!(matches!(
        gate_turn(&req(None), &state(9999, &[], false)),
        Gate::Proceed
    ));
}

#[test]
fn max_tool_rounds_zero_is_unbounded_not_instant_break() {
    // `Some(0)` 与 `None` 同义（与 `SessionConfig::max_tool_rounds` 的
    // "0 = 不限制" 契约一致），绝不能被解释成"0 轮即熔断"。
    assert_eq!(req(Some(0)).max_tool_rounds, None);
    assert!(matches!(
        gate_turn(&req(Some(0)), &state(0, &[], false)),
        Gate::Proceed
    ));
}

// ── 退出条件 ②：abort（循环顶部边界检查点）────────────────────────────

#[test]
fn abort_at_loop_boundary_exits_without_bubbling() {
    // 边界检查点用 `AbortedAtBoundary`（收尾为 Ok），不是 `Aborted`（收尾为
    // `Err(Aborted)`）：上一轮 Turn 已定稿落库，冒泡 Err 会让消费循环的
    // `persist_failure` 把成功的 Turn 误回滚为 Failed。
    assert!(matches!(
        gate_turn(&req(None), &state(1, &[], true)),
        Gate::Exit(TurnExit::AbortedAtBoundary)
    ));
}

#[test]
fn max_rounds_is_judged_before_abort() {
    // 两个退出条件同时成立时软上限优先——它带明确提示文案，
    // 比"用户中止"更能解释为什么停在这里。
    match gate_turn(&req(Some(2)), &state(2, &[], true)) {
        Gate::Exit(TurnExit::MaxToolRounds { max }) => assert_eq!(max, 2),
        other => panic!("期望 MaxToolRounds {{ max: 2 }}，实得 {other:?}"),
    }
}
