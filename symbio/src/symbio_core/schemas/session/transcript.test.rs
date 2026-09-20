//! `transcript.rs` 的单元测试（与实现同目录，见 `scripts/test-layout-audit.mjs`）。

use super::*;
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsChange, VdfsNode, VDFS_CHANGE_CREATED, VDFS_CHANGE_DELETED, VDFS_CHANGE_UPDATED,
};
use serde_json::json;

/// 造一个与 `plugins/session/plugin/nodes.rs::message_node` 同形状的消息节点。
fn node(id: &str, msg_type: &str, status: &str, content: &str, meta: Value) -> VdfsNode {
    let mut n = VdfsNode::file(id, "助手", VdfsAccess::READ);
    n.ext = Some("message".to_string());
    n.status = status.to_string();
    let _ = n.attributes.insert("role".to_string(), json!("assistant"));
    let _ = n.attributes.insert("type".to_string(), json!(msg_type));
    let _ = n.attributes.insert("meta".to_string(), meta);
    let _ = n.attributes.insert("seq".to_string(), json!(7));
    let _ = n
        .attributes
        .insert("content_len".to_string(), json!(content.len()));
    n
}

fn change(path: &str, kind: &str) -> VdfsChange {
    VdfsChange::new(path, kind)
}

#[test]
fn created_carries_full_text() {
    let mut b = TranscriptPatchBuilder::default();
    let c = change("abc/消息/m1", VDFS_CHANGE_CREATED)
        .with_node(node("m1", "text", "streaming", "你好", json!(null)))
        .with_content("你好");
    let PatchOutcome::Patch(p) = b.apply(&c) else {
        panic!("created 应产出补丁");
    };
    assert_eq!(p.id, "m1");
    assert_eq!(
        p.content.as_ref().map(MessageContent::to_text),
        Some("你好".into())
    );
    assert_eq!(p.status, Some(MessageStatus::Streaming));
    // 合并视图记的是全量
    assert_eq!(
        b.last_message("m1")
            .and_then(|m| m.content.as_ref())
            .map(MessageContent::to_text),
        Some("你好".into())
    );
}

#[test]
fn appended_yields_only_the_delta_and_accumulates() {
    let mut b = TranscriptPatchBuilder::default();
    let created = change("abc/消息/m1", VDFS_CHANGE_CREATED)
        .with_node(node("m1", "text", "streaming", "你好", json!(null)))
        .with_content("你好");
    let _ = b.apply(&created);

    let appended = VdfsChange::appended("abc/消息/m1", "，世界");
    let PatchOutcome::Patch(p) = b.apply(&appended) else {
        panic!("appended 应产出补丁");
    };
    // 热路径只带增量，不带结构
    assert_eq!(
        p.content.as_ref().map(MessageContent::to_text),
        Some("，世界".into())
    );
    assert!(p.role.is_none() && p.msg_type.is_none());
    // 合并视图累加到全量
    assert_eq!(
        b.last_message("m1")
            .and_then(|m| m.content.as_ref())
            .map(MessageContent::to_text),
        Some("你好，世界".into())
    );
}

#[test]
fn updated_with_unchanged_content_only_carries_status() {
    let mut b = TranscriptPatchBuilder::default();
    let _ = b.apply(
        &change("abc/消息/m1", VDFS_CHANGE_CREATED)
            .with_node(node("m1", "text", "streaming", "你好", json!(null)))
            .with_content("你好"),
    );

    let done = change("abc/消息/m1", VDFS_CHANGE_UPDATED)
        .with_node(node("m1", "text", "completed", "你好", json!(null)))
        .with_content("你好");
    let PatchOutcome::Patch(p) = b.apply(&done) else {
        panic!("updated 应产出补丁");
    };
    // 正文没变 ⇒ 不重复下发（否则消费端叠字）
    assert!(p.content.is_none(), "正文未变时补丁不得带 content");
    assert_eq!(p.status, Some(MessageStatus::Completed));
}

#[test]
fn updated_with_grown_content_yields_the_new_tail() {
    let mut b = TranscriptPatchBuilder::default();
    let _ = b.apply(
        &change("abc/消息/m1", VDFS_CHANGE_CREATED)
            .with_node(node("m1", "text", "streaming", "上", json!(null)))
            .with_content("上"),
    );
    let grown = change("abc/消息/m1", VDFS_CHANGE_UPDATED)
        .with_node(node("m1", "text", "streaming", "上善", json!(null)))
        .with_content("上善");
    let PatchOutcome::Patch(p) = b.apply(&grown) else {
        panic!("updated 应产出补丁");
    };
    assert_eq!(
        p.content.as_ref().map(MessageContent::to_text),
        Some("善".into())
    );
}

#[test]
fn composite_node_content_is_read_back_as_the_message() {
    let mut b = TranscriptPatchBuilder::default();
    // 组合节点的「正文」= 整条消息的 JSON 视图（`message_text`）
    let args = json!({"city": "上海"});
    let raw = ChatMessage {
        id: "tc1".to_string(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::ToolCall),
        name: Some("weather".to_string()),
        content: Some(MessageContent::Text(args.to_string())),
        ..Default::default()
    };
    let view = serde_json::to_string_pretty(&raw).unwrap();
    let c = change("abc/消息/tc1", VDFS_CHANGE_CREATED)
        .with_node(node("tc1", "tool_call", "streaming", &view, json!(null)))
        .with_content(view);
    let PatchOutcome::Patch(p) = b.apply(&c) else {
        panic!("组合节点 created 应产出补丁");
    };
    assert_eq!(p.name.as_deref(), Some("weather"));
    assert_eq!(
        p.content.as_ref().map(MessageContent::to_text),
        Some(args.to_string()),
        "工具参数不得退化成包了一层的消息 JSON"
    );
}

#[test]
fn created_without_node_view_is_flagged_not_silently_skipped() {
    let mut b = TranscriptPatchBuilder::default();
    let c = change("abc/消息/m1", VDFS_CHANGE_CREATED);
    assert!(matches!(b.apply(&c), PatchOutcome::MissingPayload));
}

#[test]
fn deleted_forgets_the_message() {
    let mut b = TranscriptPatchBuilder::default();
    let _ = b.apply(
        &change("abc/消息/m1", VDFS_CHANGE_CREATED)
            .with_node(node("m1", "text", "completed", "x", json!(null)))
            .with_content("x"),
    );
    assert!(matches!(
        b.apply(&change("abc/消息/m1", VDFS_CHANGE_DELETED)),
        PatchOutcome::Nothing
    ));
    assert!(b.last_message("m1").is_none());
}

#[test]
fn session_node_is_not_a_message() {
    let mut b = TranscriptPatchBuilder::default();
    let mut s = VdfsNode::file("abc", "会话", VdfsAccess::READ_WRITE);
    s.status = "working".to_string();
    let c = change("abc", VDFS_CHANGE_UPDATED)
        .with_node(s)
        .with_content("{}");
    // 会话节点会产出补丁，但状态词 `working` 不是消息状态 ⇒ status 缺失。
    // 调用方据 `path` 自行分流（见 subagent / CLI），折算器不把两者混成一张表。
    let PatchOutcome::Patch(p) = b.apply(&c) else {
        panic!("updated 应产出补丁");
    };
    assert!(p.status.is_none());
}
