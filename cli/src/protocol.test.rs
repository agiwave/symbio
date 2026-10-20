//! `cli/src/protocol.rs` 的单元测试。
//!
//! 钉住两条：汇报标记的读法（`exclude_from_context`，不是 `surface`），以及
//! 等待用户动作的交互卡能被识别出**缺哪种答复**——CLI 的播报据此说人话。

use super::*;
use serde_json::json;

fn interaction(status: NodeStatus, meta: Value) -> ChatNode {
    ChatNode {
        id: "res-1".to_string(),
        msg_type: Some(symbio::symbio_core::chat_node::NodeKind::Interaction),
        content: Some(MessageContent::Text("需要确认：删除文件".to_string())),
        status: Some(status),
        meta: Some(meta),
        ..Default::default()
    }
}

#[test]
fn opening_node_is_progress() {
    // 写侧形状：`chat_loop/compose.rs` 的开场白节点（重建侧 `opening_node` 同形）
    let meta = json!({"surface": "reply", "reason": "unclassified", "exclude_from_context": true});
    assert!(is_progress_surface(Some(&meta)));
}

#[test]
fn final_answer_node_is_not_progress() {
    // `Answered` 答话节点：surface 相同，只有排除标记相反
    let meta = json!({"surface": "reply", "reason": "clarify", "exclude_from_context": false});
    assert!(!is_progress_surface(Some(&meta)));
}

#[test]
fn node_without_meta_is_not_progress() {
    assert!(!is_progress_surface(None));
    assert!(!is_progress_surface(Some(&json!({"surface": "reply"}))));
}

#[test]
fn confirm_card_names_the_tool_and_the_missing_answer() {
    let node = interaction(
        NodeStatus::WaitingUserAction,
        json!({"prompt": {"kind": "confirm", "tool_name": "shell", "risk_level": "high"}}),
    );
    let card = waiting_card(&node).expect("等待用户动作的交互卡必须被识别");
    assert_eq!(card.kind, WaitKind::Confirm);
    assert_eq!(card.label(), "审批卡");
    assert_eq!(card.tool.as_deref(), Some("shell"));
    assert_eq!(card.risk.as_deref(), Some("high"));
    assert_eq!(card.missing_answer(), "通过 / 拒绝");
    assert_eq!(card.title(), "需要确认：删除文件");
}

#[test]
fn question_card_reports_the_number_of_questions() {
    let node = interaction(
        NodeStatus::WaitingUserAction,
        json!({"prompt": {"kind": "question", "questions": [{}, {}]}}),
    );
    let card = waiting_card(&node).expect("提问卡必须被识别");
    assert_eq!(card.kind, WaitKind::Question);
    assert_eq!(card.label(), "提问卡");
    assert_eq!(card.missing_answer(), "回答 2 个问题");
}

#[test]
fn a_settled_interaction_is_not_waiting() {
    let done = interaction(
        NodeStatus::Completed,
        json!({"prompt": {"kind": "confirm"}}),
    );
    assert!(waiting_card(&done).is_none(), "非等待态的卡片不再缺答复");
    let plain = ChatNode {
        status: Some(NodeStatus::WaitingUserAction),
        ..Default::default()
    };
    assert!(
        waiting_card(&plain).is_none(),
        "不是 Interaction 形状不按卡片处理"
    );
}
