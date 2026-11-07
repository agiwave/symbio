//! 完成投影的判据：全部终局才落一条完成事实；重试看最大 attempt；已落不重落。

use super::*;
use symbio_plugin_api::drive::DriveKey;

fn world(kind: &str, id: &str, payload: Value) -> DriveFact {
    DriveFact {
        kind: kind.to_string(),
        key: DriveKey {
            port: String::new(),
            id: id.to_string(),
            attempt: 0,
        },
        payload,
        at_ms: 0,
        reply: None,
    }
}

fn ring(id: &str, attempt: u32, kind: &str) -> DriveFact {
    DriveFact {
        kind: kind.to_string(),
        key: DriveKey {
            port: TOOL.to_string(),
            id: id.to_string(),
            attempt,
        },
        payload: serde_json::json!({}),
        at_ms: 0,
        reply: None,
    }
}

fn requested(request_id: &str, calls: &[&str]) -> DriveFact {
    world(
        EVENT_TOOL_REQUESTED,
        request_id,
        serde_json::json!({
            "root_id": "u-1",
            "calls": calls
                .iter()
                .map(|id| serde_json::json!({ "id": id, "name": "echo", "arguments": {} }))
                .collect::<Vec<_>>(),
        }),
    )
}

fn observe(facts: Vec<DriveFact>) -> Vec<FactDraft> {
    ToolCallsDone.observe(&TerminationView {
        facts: &facts,
        exhausted: true,
    })
}

fn debt_id(request_id: &str, call_id: &str) -> String {
    format!("tool:{request_id}:{call_id}")
}

#[test]
fn a_closed_batch_emits_one_done_fact() {
    let request_id = "tool.requested:model-1";
    let drafts = observe(vec![
        requested(request_id, &["c1", "c2"]),
        ring(&debt_id(request_id, "c1"), 1, "sent"),
        ring(&debt_id(request_id, "c1"), 1, "response"),
        ring(&debt_id(request_id, "c2"), 1, "sent"),
        ring(&debt_id(request_id, "c2"), 1, "response"),
    ]);

    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].kind, TOOL_CALLS_DONE);
    assert_eq!(drafts[0].key, format!("tool.calls.done:{request_id}"));
    assert_eq!(drafts[0].payload["requested"], request_id);
    assert_eq!(drafts[0].payload["calls"], 2);
}

/// 部分失败：判死也是终局；完成事实不因其中一笔失败而缺席。
#[test]
fn a_failed_call_still_counts_as_terminal() {
    let request_id = "tool.requested:model-1";
    let drafts = observe(vec![
        requested(request_id, &["c1", "c2"]),
        ring(&debt_id(request_id, "c1"), 1, "response"),
        ring(&debt_id(request_id, "c2"), 2, "failed"),
    ]);

    assert_eq!(drafts.len(), 1, "一笔应答 + 一笔判死 ⇒ 这批齐了");
    assert_eq!(drafts[0].payload["calls"], 2);
}

/// 一笔待批：挂起的债没有收束（`sent{deadline_ms: null}`），完成事实必须继续缺席。
#[test]
fn a_pending_call_holds_the_done_fact() {
    let request_id = "tool.requested:model-1";
    let drafts = observe(vec![
        requested(request_id, &["c1", "c2"]),
        ring(&debt_id(request_id, "c1"), 1, "response"),
        ring(&debt_id(request_id, "c2"), 1, "sent"),
    ]);

    assert!(
        drafts.is_empty(),
        "停在「等用户」的一笔未收束，这批不算完成"
    );
}

/// 一笔迟归：旧 attempt 的 response 落在投影里，也不能冒充当前 attempt 完成。
#[test]
fn a_late_return_on_a_superseded_attempt_is_not_complete() {
    let request_id = "tool.requested:model-1";
    let drafts = observe(vec![
        requested(request_id, &["c1"]),
        ring(&debt_id(request_id, "c1"), 1, "sent"),
        ring(&debt_id(request_id, "c1"), 1, "response"),
        ring(&debt_id(request_id, "c1"), 2, "sent"),
    ]);

    assert!(
        drafts.is_empty(),
        "当前 attempt 还在飞，迟归的旧 attempt 不算完成"
    );
}

/// 一笔重试：只有当前 attempt 收束才算；完成后再次观察不重复落。
#[test]
fn a_retried_call_completes_once_and_only_once() {
    let request_id = "tool.requested:model-1";
    let mut facts = vec![
        requested(request_id, &["c1"]),
        ring(&debt_id(request_id, "c1"), 1, "sent"),
        ring(&debt_id(request_id, "c1"), 2, "sent"),
        ring(&debt_id(request_id, "c1"), 2, "response"),
    ];

    let drafts = observe(facts.clone());
    assert_eq!(drafts.len(), 1);
    facts.push(world(
        TOOL_CALLS_DONE,
        &format!("tool.calls.done:{request_id}"),
        drafts[0].payload.clone(),
    ));
    assert!(
        observe(facts).is_empty(),
        "完成事实按 request_id 幂等，不重复落"
    );
}
