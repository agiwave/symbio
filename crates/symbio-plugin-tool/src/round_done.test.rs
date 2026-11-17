//! 完成行的判据：一个未落边界的请求欠一条完成事实；`after_port(TOOL)` 决定它还的时机；
//! 同一请求已落 ⇒ 不再欠。

use super::*;
use symbio_plugin_api::debt::DriveRequest;

fn request() -> DriveRequest {
    DriveRequest {
        session_id: "s-1".to_string(),
        provider: None,
        principal: "session/main".to_string(),
    }
}

fn note_requested(facts: &mut DebtFacts, request_id: &str, calls: &[&str]) {
    facts.note_fact_payload(
        EVENT_TOOL_REQUESTED,
        request_id,
        json!({
            "root_id": "u-1",
            "calls": calls
                .iter()
                .map(|id| json!({ "id": id, "name": "echo", "arguments": {} }))
                .collect::<Vec<_>>(),
        }),
    );
}

fn row_request<'a>(key: &'a [(String, String)]) -> RowRequest<'a> {
    RowRequest {
        row: TOOL_CALLS_DONE_ROW,
        key,
        anchor: "anchor",
        now_ms: 0,
        principal: "session/main",
        user_seq: 7,
        facts: &[],
        history: &[],
        after_port: Some(TOOL),
    }
}

#[test]
fn an_undeclared_boundary_owes_one_done_row() {
    let request_id = "tool.requested:model-1";
    let mut facts = DebtFacts::default();
    note_requested(&mut facts, request_id, &["c1", "c2"]);

    let rows = ToolCallsDoneDebts.rows(&request(), &facts);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, TOOL_CALLS_DONE_ROW);
    assert_eq!(rows[0].port, PLUGIN_ROW);
    assert_eq!(
        rows[0].after_port.as_deref(),
        Some(TOOL),
        "完成事实只在工具端口当圈没人认领时才落"
    );
    assert!(rows[0].unless.is_some(), "已落边界的请求不再欠");
}

#[test]
fn a_written_boundary_removes_the_row() {
    let request_id = "tool.requested:model-1";
    let mut facts = DebtFacts::default();
    note_requested(&mut facts, request_id, &["c1"]);
    facts.note_fact_payload(TOOL_CALLS_DONE, &done_key(request_id), json!({}));

    assert!(
        ToolCallsDoneDebts.rows(&request(), &facts).is_empty(),
        "完成事实按 request_id 幂等"
    );
}

#[test]
fn a_request_without_calls_owes_nothing() {
    let mut facts = DebtFacts::default();
    note_requested(&mut facts, "tool.requested:model-1", &[]);

    assert!(
        ToolCallsDoneDebts.rows(&request(), &facts).is_empty(),
        "没有工具调用就没有轮边界"
    );
}

#[test]
fn the_row_writes_one_done_fact() {
    let request_id = "tool.requested:model-1";
    let key = vec![
        ("request".to_string(), request_id.to_string()),
        ("calls".to_string(), "2".to_string()),
    ];

    let RowStep::Facts(drafts) = ToolCallsDoneRow.run(&row_request(&key)) else {
        panic!("完成行必须落事实");
    };

    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].kind, TOOL_CALLS_DONE);
    assert_eq!(drafts[0].key, format!("tool.calls.done:{request_id}"));
    assert_eq!(drafts[0].payload["requested"], request_id);
    assert_eq!(drafts[0].payload["calls"], 2);
    assert_eq!(drafts[0].produced_by, Some(7), "完成事实的溯源锚是行调用者");
}

#[test]
fn the_row_ignores_an_empty_call_list() {
    let key = vec![
        ("request".to_string(), "tool.requested:model-1".to_string()),
        ("calls".to_string(), "0".to_string()),
    ];

    let RowStep::Facts(drafts) = ToolCallsDoneRow.run(&row_request(&key)) else {
        panic!("完成行必须返回事实步");
    };
    assert!(drafts.is_empty());
}

#[test]
fn the_row_declares_the_done_fact_as_its_write() {
    assert_eq!(ToolCallsDoneRow.writes(), &[TOOL_CALLS_DONE]);
    assert_eq!(ToolCallsDoneRow.row(), TOOL_CALLS_DONE_ROW);
    assert_eq!(ToolCallsDoneRow.owner(), PLUGIN_ID_TOOL);
}
