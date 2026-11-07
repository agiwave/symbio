//! 工具债行的行为判据：一条标记一笔债、id 从 `(request_id, call_id)` 派生、
//! 恢复按 `call_id` 一对一。

use super::*;
use symbio_plugin_api::debt::DriveRequest;

fn request() -> DriveRequest {
    DriveRequest {
        session_id: "s-1".to_string(),
        provider: None,
        principal: "session/main".to_string(),
    }
}

fn note_marker(facts: &mut DebtFacts, request_id: &str, call_id: &str) {
    let id = format!("tool.call.requested:{request_id}:{call_id}");
    facts.note_fact(EVENT_TOOL_CALL_REQUESTED, &id);
    facts.note_fact_payload(
        EVENT_TOOL_CALL_REQUESTED,
        &id,
        json!({ "request_id": request_id, "call_id": call_id }),
    );
}

fn note_answer(facts: &mut DebtFacts, call_id: &str) {
    let id = format!("user-answer:{call_id}");
    facts.note_fact(EVENT_USER_ANSWER, &id);
    facts.note_fact_payload(
        EVENT_USER_ANSWER,
        &id,
        json!({ "call_id": call_id, "kind": "answer" }),
    );
}

fn call_rows(rows: &[DebtSpec]) -> Vec<&DebtSpec> {
    rows.iter().filter(|row| row.name == "tool-call").collect()
}

fn resume_rows(rows: &[DebtSpec]) -> Vec<&DebtSpec> {
    rows.iter()
        .filter(|row| row.name == "tool-resume")
        .collect()
}

#[test]
fn no_marker_means_no_debt() {
    let facts = DebtFacts::default();
    let rows = ToolDebts.rows(&request(), &facts);
    assert!(rows.is_empty(), "缺标记就不欠");
}

#[test]
fn one_marker_is_one_debt_keyed_by_request_and_call() {
    let mut facts = DebtFacts::default();
    note_marker(&mut facts, "tool.requested:r1", "call-1");

    let rows = ToolDebts.rows(&request(), &facts);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.port, TOOL);
    assert_eq!(row.id, "tool:tool.requested:r1:call-1");
    assert_eq!(
        row.payload,
        json!({ "request_id": "tool.requested:r1", "call_id": "call-1" }),
        "载荷把 (request_id, call_id) 一起交给执行体"
    );
    assert_eq!(
        row.when.as_deref(),
        Some(EVENT_TOOL_CALL_REQUESTED),
        "标记种类触发；身份由这条行本身承载（行是逐条标记生成的）"
    );
    assert!(row.when_id.is_none(), "单笔请求没有派发链前置");
    assert!(row.after.is_none(), "不绑收束事实（收束会把挂起变成堵点）");
    assert_eq!((row.lease_ms, row.max_attempts), (600_000, 2));
}

#[test]
fn n_calls_in_one_request_are_n_debts() {
    let mut facts = DebtFacts::default();
    note_marker(&mut facts, "tool.requested:r1", "call-1");
    note_marker(&mut facts, "tool.requested:r1", "call-2");
    note_marker(&mut facts, "tool.requested:r1", "call-3");

    let rows = ToolDebts.rows(&request(), &facts);
    let calls = call_rows(&rows);
    assert_eq!(calls.len(), 3, "一调用一笔债，环上没有「批」");
    assert_eq!(
        calls.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec![
            "tool:tool.requested:r1:call-1",
            "tool:tool.requested:r1:call-2",
            "tool:tool.requested:r1:call-3",
        ]
    );
    assert!(calls[0].when_id.is_none(), "第一笔没有前置");
    assert_eq!(
        calls[1].when_id,
        Some((
            DRIVE_SENT.to_string(),
            "tool:tool.requested:r1:call-1".to_string()
        )),
        "第二笔等第一笔**已派发**（sent 事实），不等它收束"
    );
    assert_eq!(
        calls[2].when_id,
        Some((
            DRIVE_SENT.to_string(),
            "tool:tool.requested:r1:call-2".to_string()
        ))
    );
    assert!(
        calls.iter().all(|row| row.after.is_none()),
        "链不绑收束事实：一笔待批不能堵住后面的调用"
    );
}

#[test]
fn the_same_call_id_under_two_requests_is_two_debts() {
    let mut facts = DebtFacts::default();
    note_marker(&mut facts, "tool.requested:r1", "call-1");
    note_marker(&mut facts, "tool.requested:r2", "call-1");

    let rows = ToolDebts.rows(&request(), &facts);
    assert_eq!(
        call_rows(&rows)
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "tool:tool.requested:r1:call-1",
            "tool:tool.requested:r2:call-1"
        ],
        "call_id 只在本请求内唯一，跨请求复用仍是两笔"
    );
    assert!(
        call_rows(&rows)
            .iter()
            .all(|row| row.when_id.is_none() && row.after.is_none()),
        "链只在请求内串：两条请求的第一笔互不等待"
    );
}

#[test]
fn resume_pairs_the_answer_to_the_call_it_names() {
    let mut facts = DebtFacts::default();
    note_marker(&mut facts, "tool.requested:r1", "call-1");
    note_marker(&mut facts, "tool.requested:r2", "call-2");
    note_answer(&mut facts, "call-1");

    let rows = ToolDebts.rows(&request(), &facts);
    let resume = resume_rows(&rows);
    assert_eq!(resume.len(), 1);
    assert_eq!(
        resume[0].id, "tool:tool.requested:r1:call-1",
        "答复属于 call-1 就恢复 call-1——更近的 r2 不是配对"
    );
    assert_eq!(resume[0].payload["answer_event"], "user-answer:call-1");
    assert_eq!(resume[0].payload["call_id"], "call-1");
    assert_eq!(resume[0].payload["request_id"], "tool.requested:r1");
}

#[test]
fn each_answer_resumes_only_its_own_call() {
    let mut facts = DebtFacts::default();
    note_marker(&mut facts, "tool.requested:r1", "call-1");
    note_marker(&mut facts, "tool.requested:r1", "call-2");
    note_answer(&mut facts, "call-1");

    let rows = ToolDebts.rows(&request(), &facts);
    let resume = resume_rows(&rows);
    assert_eq!(resume.len(), 1, "同一请求的两张卡各欠各的恢复");
    assert_eq!(resume[0].id, "tool:tool.requested:r1:call-1");

    note_answer(&mut facts, "call-2");
    let rows = ToolDebts.rows(&request(), &facts);
    let resume = resume_rows(&rows);
    assert_eq!(
        resume.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec![
            "tool:tool.requested:r1:call-1",
            "tool:tool.requested:r1:call-2"
        ],
        "第二条答复恢复第二张卡，不覆盖第一条"
    );
    assert!(resume[0].when_id.is_none());
    assert_eq!(
        resume[1].when_id,
        Some((
            DRIVE_SENT.to_string(),
            "tool:tool.requested:r1:call-1".to_string()
        )),
        "恢复也走同一条派发链：前一笔已派发，下一笔才恢复"
    );
}

#[test]
fn an_unowned_answer_does_not_hijack_another_call() {
    let mut facts = DebtFacts::default();
    note_marker(&mut facts, "tool.requested:r1", "call-1");
    note_answer(&mut facts, "call-gone");

    let rows = ToolDebts.rows(&request(), &facts);
    assert!(
        resume_rows(&rows).is_empty(),
        "找不到所属标记就不恢复，不拿别的调用顶替"
    );
}

#[test]
fn the_chain_waits_for_dispatch_not_closing() {
    let mut facts = DebtFacts::default();
    note_marker(&mut facts, "tool.requested:r1", "call-1");
    note_marker(&mut facts, "tool.requested:r1", "call-2");
    note_marker(&mut facts, "tool.requested:r1", "call-3");

    let rows = ToolDebts.rows(&request(), &facts);
    let calls = call_rows(&rows);
    let third = calls[2];
    assert!(third.after.is_none(), "链不得落到收束事实");
    assert_eq!(
        third.when_id,
        Some((
            DRIVE_SENT.to_string(),
            "tool:tool.requested:r1:call-2".to_string()
        )),
        "第 3 笔只等第 2 笔**已派发**"
    );

    // 环写口的账：call-1 已派发并收束；call-2 已派发但挂起
    // （`sent{deadline_ms:null}`，**没有** `response`/`failed`）；call-3 未派发。
    let ledger = [
        ("sent", "tool:tool.requested:r1:call-1"),
        ("response", "tool:tool.requested:r1:call-1"),
        ("sent", "tool:tool.requested:r1:call-2"),
    ];
    let previous = "tool:tool.requested:r1:call-2";
    let dispatched = |id: &str| {
        ledger
            .iter()
            .any(|(kind, fact_id)| kind == &DRIVE_SENT && fact_id == &id)
    };
    let closed = |id: &str| {
        ledger
            .iter()
            .any(|(kind, fact_id)| (*kind == "response" || *kind == "failed") && fact_id == &id)
    };
    assert!(dispatched(previous), "派发链的前置已满足：第 3 笔可派发");
    assert!(
        !closed(previous),
        "第 2 笔没有收束事实：改回 `after(前一债 id)`（经 closing_key 落到收束事实）时，\
         第 3 笔将永远选不到——这条用例就是那个退化的红线"
    );
}

#[test]
fn multiple_answers_for_one_call_keep_the_last() {
    let mut facts = DebtFacts::default();
    note_marker(&mut facts, "tool.requested:r1", "call-1");
    note_answer(&mut facts, "call-1");
    // 同一调用的第二次答复：只恢复一次，取最后一条。
    facts.note_fact(EVENT_USER_ANSWER, "user-answer:call-1:again");
    facts.note_fact_payload(
        EVENT_USER_ANSWER,
        "user-answer:call-1:again",
        json!({ "call_id": "call-1", "kind": "answer" }),
    );

    let rows = ToolDebts.rows(&request(), &facts);
    let resume = resume_rows(&rows);
    assert_eq!(resume.len(), 1);
    assert_eq!(
        resume[0].payload["answer_event"],
        "user-answer:call-1:again"
    );
}
