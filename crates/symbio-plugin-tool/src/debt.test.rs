//! 工具债行的行为判据：有事实才欠、id 从事实身份派生、载荷带触发事实 id。

use super::*;
use symbio_plugin_api::debt::DriveRequest;

fn request() -> DriveRequest {
    DriveRequest {
        session_id: "s-1".to_string(),
        provider: None,
        principal: "session/main".to_string(),
    }
}

#[test]
fn no_tool_requested_means_no_debt() {
    let facts = DebtFacts::default();
    let rows = ToolDebts.rows(&request(), &facts);
    assert!(rows.is_empty(), "缺这条事实就不欠");
}

#[test]
fn the_debt_id_derives_from_the_latest_requested_fact() {
    let mut facts = DebtFacts::default();
    facts.note_fact(EVENT_TOOL_REQUESTED, "tool.requested:earlier");
    facts.note_fact("artifact.added", "artifact:x");
    facts.note_fact(EVENT_TOOL_REQUESTED, "tool.requested:latest");

    let rows = ToolDebts.rows(&request(), &facts);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.port, TOOL);
    assert_eq!(
        row.id, "tool:tool.requested:latest",
        "身份来自事实，不靠计数"
    );
    assert_eq!(
        row.payload,
        serde_json::json!({ "request_id": "tool.requested:latest" }),
        "载荷带触发事实 id：执行体据此精确配对"
    );
    assert_eq!(row.when.as_deref(), Some(EVENT_TOOL_REQUESTED));
    assert!(row.unless.is_none());
    assert!(row.when_no_open.is_none());
}

fn note_request(facts: &mut DebtFacts, id: &str, calls: &[&str]) {
    facts.note_fact(EVENT_TOOL_REQUESTED, id);
    let calls: Vec<_> = calls
        .iter()
        .map(|call| json!({ "id": call, "name": "demo", "arguments": {} }))
        .collect();
    facts.note_fact_payload(EVENT_TOOL_REQUESTED, id, json!({ "calls": calls }));
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

fn resume_rows(rows: &[DebtSpec]) -> Vec<&DebtSpec> {
    rows.iter()
        .filter(|row| row.name == "tool-resume")
        .collect()
}

#[test]
fn resume_pairs_the_answer_to_the_request_that_owns_its_call() {
    let mut facts = DebtFacts::default();
    note_request(&mut facts, "tool.requested:r1", &["call-1"]);
    note_request(&mut facts, "tool.requested:r2", &["call-2"]);
    note_answer(&mut facts, "call-1");

    let rows = ToolDebts.rows(&request(), &facts);
    let resume = resume_rows(&rows);
    assert_eq!(resume.len(), 1);
    assert_eq!(
        resume[0].id, "tool:tool.requested:r1",
        "答复属于 r1 就恢复 r1——最近一条请求 r2 不是配对"
    );
    assert_eq!(
        resume[0].payload["answer_event"], "user-answer:call-1",
        "载荷带答复事实自己的身份"
    );
}

#[test]
fn each_answer_resumes_its_own_request() {
    let mut facts = DebtFacts::default();
    note_request(&mut facts, "tool.requested:r1", &["call-1"]);
    note_request(&mut facts, "tool.requested:r2", &["call-2"]);
    note_answer(&mut facts, "call-1");
    note_answer(&mut facts, "call-2");

    let rows = ToolDebts.rows(&request(), &facts);
    let resume = resume_rows(&rows);
    assert_eq!(resume.len(), 2, "两条答复各自归自己的请求");
    assert_eq!(resume[0].id, "tool:tool.requested:r1");
    assert_eq!(resume[0].payload["answer_event"], "user-answer:call-1");
    assert_eq!(resume[1].id, "tool:tool.requested:r2");
    assert_eq!(resume[1].payload["answer_event"], "user-answer:call-2");
}

#[test]
fn an_unowned_answer_does_not_hijack_the_latest_request() {
    let mut facts = DebtFacts::default();
    note_request(&mut facts, "tool.requested:r1", &["call-1"]);
    note_answer(&mut facts, "call-gone");

    let rows = ToolDebts.rows(&request(), &facts);
    assert!(
        resume_rows(&rows).is_empty(),
        "找不到所属请求就不恢复，不拿最新请求顶替"
    );
}

#[test]
fn multiple_answers_for_one_request_keep_the_last() {
    let mut facts = DebtFacts::default();
    note_request(&mut facts, "tool.requested:r1", &["call-1", "call-2"]);
    note_answer(&mut facts, "call-1");
    note_answer(&mut facts, "call-2");

    let rows = ToolDebts.rows(&request(), &facts);
    let resume = resume_rows(&rows);
    assert_eq!(resume.len(), 1, "一条请求只有一个恢复键，不重复申报");
    assert_eq!(resume[0].id, "tool:tool.requested:r1");
    assert_eq!(
        resume[0].payload["answer_event"], "user-answer:call-2",
        "一次恢复只消费一条答复，取最后一条；批内多卡另案（逐调用债务）"
    );
}
