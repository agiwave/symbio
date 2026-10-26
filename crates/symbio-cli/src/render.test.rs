//! `crates/symbio-cli/src/render.rs` 的单元测试 —— 与实现同级分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 汇报标记的判定已收口到 `protocol.rs`（唯一协议读法），其判据由
//! `protocol.test.rs` 钉住；本文件保留同一条判据的回归口径。

use crate::protocol::{is_progress_surface, waiting_card};
use crate::render::{waiting_lines, Renderer};
use serde_json::json;
use symbio::symbio_core::chat_node::{ChatNode, MessageContent, NodeKind, NodeStatus};

fn interaction(status: NodeStatus) -> ChatNode {
    ChatNode {
        id: "res-1".to_string(),
        msg_type: Some(NodeKind::Interaction),
        content: Some(MessageContent::Text("需要确认：删除文件".to_string())),
        status: Some(status),
        meta: Some(json!({
            "prompt": {"kind": "confirm", "tool_name": "shell", "risk_level": "high"}
        })),
        ..Default::default()
    }
}

fn question() -> ChatNode {
    ChatNode {
        id: "res-2".to_string(),
        msg_type: Some(NodeKind::Interaction),
        content: Some(MessageContent::Text("请选择运行环境".to_string())),
        status: Some(NodeStatus::WaitingUserAction),
        meta: Some(json!({
            "prompt": {
                "kind": "question",
                "questions": [{"text": "环境？"}, {"text": "端口？"}]
            }
        })),
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
fn a_waiting_card_marks_the_turn_unfinished_until_it_settles() {
    let mut renderer = Renderer::new(true, false, "auto");
    renderer.begin_turn();

    renderer.on_message(&interaction(NodeStatus::WaitingUserAction));
    assert!(
        renderer.saw_waiting_action(),
        "等待用户动作的卡片必须让本轮被判为未完成"
    );

    // 桌面端在同一轮里作答之后，卡片收束为 Completed —— 本轮不再是「停在半路」。
    renderer.on_message(&interaction(NodeStatus::Completed));
    assert!(
        !renderer.saw_waiting_action(),
        "卡片收束后未完成标记必须撤掉"
    );
}

#[test]
fn auto_mode_names_the_confirm_card_and_why_it_cannot_continue() {
    let node = interaction(NodeStatus::WaitingUserAction);
    let card = waiting_card(&node).expect("确认卡应被识别为等待用户动作");
    let text = waiting_lines(&card, true, false).join("\n");

    assert!(text.contains("审批卡"), "应说明是哪张卡：{text}");
    assert!(text.contains("通过 / 拒绝"), "应说明缺哪种答复：{text}");
    assert!(
        text.contains("shell") && text.contains("high"),
        "应带工具与风险等级：{text}"
    );
    assert!(
        text.contains("不会自动继续"),
        "应说明为什么不自动继续：{text}"
    );
}

#[test]
fn auto_mode_names_the_question_card_and_the_number_of_answers() {
    let card = waiting_card(&question()).expect("提问卡应被识别为等待用户动作");
    let text = waiting_lines(&card, true, false).join("\n");

    assert!(text.contains("提问卡"), "应说明是哪张卡：{text}");
    assert!(text.contains("回答 2 个问题"), "应说明缺几个答复：{text}");
    assert!(
        text.contains("不会自动继续"),
        "应说明为什么不自动继续：{text}"
    );
}

#[test]
fn quiet_mode_still_reports_the_waiting_terminal_state() {
    let card = waiting_card(&interaction(NodeStatus::WaitingUserAction))
        .expect("确认卡应被识别为等待用户动作");
    let lines = waiting_lines(&card, true, true);

    assert_eq!(lines.len(), 1, "静默模式只留一行终态：{lines:?}");
    assert!(
        lines[0].contains("auto 模式无人作答，本轮未完成"),
        "{lines:?}"
    );
}
