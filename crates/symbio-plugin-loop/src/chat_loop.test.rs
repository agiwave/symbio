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
//! （[D-073](../../../docs/003-design/014-单环一表.md#d-073-turnstate-的轮内簿记与闸门等工具齐一并退场闸门只剩退出条件 "DE-014-单环一表")），「工具轮软上限」随它恒 0 的计数一起退役（[D-074](../../../docs/003-design/014-单环一表.md#d-074-恒-0-的轮内计数退出主循环工具轮软上限整条退役抽干点搬到批次边界 "DE-014-单环一表")）——
//! 工具轮数在这一层不可见，本文件因此**不该**再出现任何按轮次的判定。
//!
//! 本文件另有第二段：收尾（[`finish_turn`]）的**分支标注补发**——它同样是主循环骨架的
//! 一段，判据是「发帧范围」，不是标注内容本身（内容在 `projection::branch` 钉）。

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

// ── 收尾的分支标注补发（[D-173]）─────────────────────────────────────────
//
// 标注本身对不对由 `symbio_core::projection::branch` 的用例钉；本段只钉**发帧范围**：
// 补发的是**一组**的成员，不是「新支那一条」。少发旧支那条，界面上就是「一支有
// `‹ ›`、另一支没有」——而切换器要两支都能点才算存在。

use super::super::test_support::{default_settings, loop_session, turn_spec, FakeSessionPort};
use async_trait::async_trait;
use serde_json::json;
use std::sync::{Arc, Mutex};
use symbio_core::{EventStore, ExecTranscriptWriter};

/// 帧口：记下每一条发出去的节点 id。
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

#[async_trait]
impl ExecTranscriptWriter for Recorder {
    async fn apply(&self, m: ChatNode) {
        self.0.lock().unwrap().push(m.id);
    }
}

/// 一条**已盖章**的成员根（盖章形状 = 重建的输出；投影本身在 core 的分支用例里钉）。
fn member(id: &str, index: usize, selected: bool) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        meta: Some(json!({
            "branch": {
                "group": "t1", "candidates": ["t1", "t2"],
                "index": index, "total": 2, "selected": selected,
            }
        })),
        ..Default::default()
    }
}

async fn context_with(messages: Vec<ChatNode>) -> SessionContext {
    let facts = Arc::new(EventStore::new());
    let port = Arc::new(FakeSessionPort::new(messages.clone()));
    let spec = turn_spec(
        "s-branch",
        std::env::temp_dir(),
        std::env::temp_dir(),
        default_settings(),
    );
    let session = Arc::new(loop_session(spec, facts, port));
    SessionContext {
        messages,
        session,
        consolidated: None,
    }
}

/// 新支落库后收尾：组里**两条**成员各发一条帧（旧支的 `selected` 也换了，它得重新出场）。
#[tokio::test]
async fn a_retry_turn_end_republishes_every_member_of_the_group() {
    let rec = Arc::new(Recorder::default());
    let ctx = context_with(vec![member("t1", 0, false), member("t2", 1, true)]).await;
    republish_branch_group(&ctx, &ExecEventSink::direct(rec.clone()), "t2").await;
    assert_eq!(
        *rec.0.lock().unwrap(),
        vec!["t1".to_string(), "t2".to_string()],
        "补发按组内顺序覆盖每一条成员"
    );
}

/// 这一轮**没开出组**（新支没产出节点就被中止，或读回来那条没有标注）⇒ 一条都不发。
///
/// 反面是「顺手把整份转写重发一遍」：那不是补发标注，是给每个节点开一次无谓的替换。
#[tokio::test]
async fn a_branchless_retry_end_republishes_nothing() {
    let rec = Arc::new(Recorder::default());
    let plain = ChatNode {
        id: "t9".to_string(),
        ..Default::default()
    };
    let ctx = context_with(vec![plain]).await;
    republish_branch_group(&ctx, &ExecEventSink::direct(rec.clone()), "t9").await;
    assert!(rec.0.lock().unwrap().is_empty(), "无组 ⇒ 不发帧");
}
