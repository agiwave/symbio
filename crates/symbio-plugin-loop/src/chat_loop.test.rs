//! `chat_loop.rs`（主循环骨架）的单元测试 —— 收尾的分支标注补发。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着**被测试的实现文件**走——
//! `state.rs` 的测试在 `chat_loop/state.test.rs`，`inputs.rs` 的在
//! `chat_loop/inputs.test.rs`；`turn.rs` / `compress.rs` / `io.rs` 暂无测试。
//!
//! 判据是「发帧范围」，不是标注内容本身（内容在 `projection::branch` 钉）。

use super::*;

// ── 收尾的分支标注补发（[D-173]）─────────────────────────────────────────
//
// 标注本身对不对由 `symbio_sdk::projection::branch` 的用例钉；本段只钉**发帧范围**：
// 补发的是**一组**的成员，不是「新支那一条」。少发旧支那条，界面上就是「一支有
// `‹ ›`、另一支没有」——而切换器要两支都能点才算存在。

use super::super::test_support::{
    default_settings, session_port_of, turn_spec, FakeSessionPort,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::{Arc, Mutex};
use symbio_kernel::exec::ExecTranscriptWriter;
use symbio_kernel::store::EventStore;

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
    let view = spec.view();
    let session = session_port_of(port);
    SessionContext {
        messages,
        view,
        facts: Some(facts as Arc<dyn symbio_kernel::store::Store<Event = symbio_kernel::Event>>),
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
