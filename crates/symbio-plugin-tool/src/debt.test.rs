//! 工具债行的行为判据：有事实才欠、id 从事实身份派生、载荷留空。

use super::*;
use symbio_plugin_api::debt::DriveRequest;

fn request() -> DriveRequest {
    DriveRequest {
        scope: 7,
        session_id: "s-1".to_string(),
        provider: None,
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
    assert_eq!(row.id, "tool:tool.requested:latest", "身份来自事实，不靠计数");
    assert_eq!(row.payload, serde_json::json!({}), "载荷留空：清单由执行体自读");
    assert_eq!(row.when.as_deref(), Some(EVENT_TOOL_REQUESTED));
    assert!(row.unless.is_none());
    assert!(row.when_no_open.is_none());
}
