//! `turn` 的**纯判据**：地址形状 / 正文累积 / 重同步标记。
//!
//! 这三件事都不需要跑一轮会话就能判——它们是「一帧该不该认」「认了怎么落」
//! 「这帧是不是重同步指令」，而这三条正是回归缺陷（回复永远是「无响应」）
//! 背后真正会写错的地方。协议舞步（订阅 / 开闸 / 入队 / 摘除）由 e2e 覆盖，
//! 这里不重复造一个假容器。

use super::*;
use crate::symbio_core::chat_node::{ChatNode, MessageContent, NodeKind, NodeRole, NodeStatus};
use crate::symbio_core::{EVENT_BUS_KIND_SYSTEM, EVENT_BUS_RESYNC_MARKER_TYPE};

const SID: &str = "chat-1";
const SESSION_ADDR: &str = "root/session/chat-1";

/// 身份写 `principal`（D-147）：夹具模仿的是**会话广播面**的帧，`role` 只是读侧残留格。
fn principal_for(role: NodeRole) -> crate::symbio_core::principal::Principal {
    use crate::symbio_core::principal::Principal;
    match role {
        NodeRole::User => Principal::from_legacy(crate::symbio_core::PRINCIPAL_USER),
        NodeRole::Assistant => Principal::from_legacy("session/main"),
        NodeRole::Tool => Principal::from_legacy("legacy/unknown"),
        NodeRole::System => Principal::from_legacy("system/local"),
    }
}

/// 一条变更（带载荷），地址与正文由参数给。
fn change(path: &str, data: Value) -> VdfsChange {
    VdfsChange::with_data(path, data)
}

/// 一条助手正文消息。
fn assistant(id: &str, status: NodeStatus, content: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        principal: Some(principal_for(NodeRole::Assistant)),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(content.to_string())),
        status: Some(status),
        ..Default::default()
    }
}

// ==================== 地址形状 ====================

#[test]
fn session_node_itself_is_not_a_message() {
    // 会话节点自身归运行态，不归消息面——认错了会让「会话离开 working」被当成一条消息。
    let c = change(SESSION_ADDR, serde_json::json!({}));
    assert!(message_at(SESSION_ADDR, &c).is_none());
}

#[test]
fn message_dir_is_not_a_message_item() {
    // 集合目录（`<sid>/message`）不带身份：身份在**地址末段**。
    let c = change(
        &format!("{SESSION_ADDR}/{SEG_MESSAGES}"),
        serde_json::json!({}),
    );
    assert!(message_at(SESSION_ADDR, &c).is_none());
}

#[test]
fn message_item_is_recognised_and_identity_comes_from_the_address() {
    // 载荷里的 `id` 是同一事实的复述；地址才是权威（两者不一致时以地址为准）。
    let c = change(
        &format!("{SESSION_ADDR}/{SEG_MESSAGES}/m1"),
        serde_json::json!({ "id": "载荷里的旧 id", "role": "assistant" }),
    );
    let m = message_at(SESSION_ADDR, &c).expect("`<会话>/message/<mid>` 是一条消息");
    assert_eq!(m.id, "m1");
}

#[test]
fn deeper_than_a_message_item_is_not_a_message() {
    let c = change(
        &format!("{SESSION_ADDR}/{SEG_MESSAGES}/m1/child"),
        serde_json::json!({ "id": "m1" }),
    );
    assert!(message_at(SESSION_ADDR, &c).is_none());
}

#[test]
fn other_collections_are_not_messages() {
    // 收件箱 / 记忆 / 工作目录都长成 `<会话>/<集合段>/<项 id>`——本判据只认「消息」
    // 那一段，这样新增一类集合时它不需要改。
    for seg in [SEG_INBOX, "memory", "workdir"] {
        let c = change(
            &format!("{SESSION_ADDR}/{seg}/x1"),
            serde_json::json!({ "id": "x1" }),
        );
        assert!(
            message_at(SESSION_ADDR, &c).is_none(),
            "集合段 `{seg}` 不该被当成消息"
        );
    }
}

#[test]
fn another_sessions_message_is_not_ours() {
    let c = change(
        "root/session/other/message/m1",
        serde_json::json!({ "id": "m1" }),
    );
    assert!(message_at(SESSION_ADDR, &c).is_none());
}

#[test]
fn message_without_payload_is_not_a_message() {
    // 资源信号（标题 / metadata / 创建）是无载荷的——解不出正文就不该造一条消息。
    let c = VdfsChange::bare(format!("{SESSION_ADDR}/{SEG_MESSAGES}/m1"));
    assert!(message_at(SESSION_ADDR, &c).is_none());
}

// ==================== 正文累积 ====================

#[test]
fn full_frame_replaces_and_delta_appends() {
    let mut acc = Accumulator::default();
    acc.apply(assistant("m1", NodeStatus::Streaming, "你"));
    acc.apply(assistant("m1", NodeStatus::Streaming, "你好"));
    // 纯增量：追加
    acc.apply(ChatNode {
        id: "m1".into(),
        principal: Some(principal_for(NodeRole::Assistant)),
        msg_type: Some(NodeKind::Body),
        delta: Some("，世界".into()),
        status: Some(NodeStatus::Streaming),
        ..Default::default()
    });
    acc.apply(assistant("m1", NodeStatus::Completed, "你好，世界"));
    assert_eq!(acc.final_text(), "你好，世界");
}

#[test]
fn status_only_frame_does_not_clobber_text() {
    // 状态帧既无 `content` 又无 `delta`——若把它当「全量帧」落，正文会被清空。
    let mut acc = Accumulator::default();
    acc.apply(assistant("m1", NodeStatus::Streaming, "已经攒好的正文"));
    acc.apply(ChatNode {
        id: "m1".into(),
        principal: Some(principal_for(NodeRole::Assistant)),
        msg_type: Some(NodeKind::Body),
        status: Some(NodeStatus::Completed),
        ..Default::default()
    });
    assert_eq!(acc.final_text(), "已经攒好的正文");
}

#[test]
fn removed_message_drops_out_of_the_accumulator() {
    let mut acc = Accumulator::default();
    acc.apply(assistant("m1", NodeStatus::Completed, "会被删掉"));
    acc.apply(ChatNode {
        id: "m1".into(),
        principal: Some(principal_for(NodeRole::Assistant)),
        status: Some(NodeStatus::Removed),
        ..Default::default()
    });
    assert!(acc.final_text().is_empty());
    assert!(!acc.known("m1"));
}

#[test]
fn last_completed_assistant_text_wins_over_a_later_unfinished_one() {
    // 「最后一条**已完成**」而不是「最后处理到的文本」：流式中的半截正文不能被
    // 当成最终答复（与 `agent_run` 的转播桥同一判据）。
    let mut acc = Accumulator::default();
    acc.apply(assistant("m1", NodeStatus::Completed, "答复一"));
    acc.apply(assistant("m2", NodeStatus::Streaming, "答复二的半截"));
    assert_eq!(acc.final_text(), "答复一");
}

#[test]
fn falls_back_to_the_last_text_when_nothing_completed() {
    let mut acc = Accumulator::default();
    acc.apply(assistant("m1", NodeStatus::Streaming, "只有这一条"));
    assert_eq!(acc.final_text(), "只有这一条");
}

#[test]
fn reasoning_and_user_messages_never_enter_the_reply() {
    // 推理独白与用户自己的消息都不在同一个地址空间里的「答复」上。
    let mut acc = Accumulator::default();
    acc.apply(ChatNode {
        id: "r1".into(),
        principal: Some(principal_for(NodeRole::Assistant)),
        msg_type: Some(NodeKind::Notice),
        meta: Some(serde_json::json!({"style_tag": "reasoning"})),
        content: Some(MessageContent::Text("思考过程".into())),
        status: Some(NodeStatus::Completed),
        ..Default::default()
    });
    acc.apply(ChatNode {
        id: "u1".into(),
        principal: Some(principal_for(NodeRole::User)),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text("用户说的话".into())),
        status: Some(NodeStatus::Completed),
        ..Default::default()
    });
    assert!(acc.final_text().is_empty());
}

#[test]
fn delta_for_an_unknown_identity_still_accumulates() {
    // 订阅晚于流开始时（回读也拿不到基线）攒下来的部分仍好过丢掉整条。
    let mut acc = Accumulator::default();
    assert!(!acc.known("m1"));
    acc.apply(ChatNode {
        id: "m1".into(),
        principal: Some(principal_for(NodeRole::Assistant)),
        msg_type: Some(NodeKind::Body),
        delta: Some("半截".into()),
        status: Some(NodeStatus::Streaming),
        ..Default::default()
    });
    assert!(acc.known("m1"));
    assert_eq!(acc.final_text(), "半截");
}

#[test]
fn aborted_turn_still_yields_its_text() {
    // 中止是**终态**（用户按了停止）：正文照给，与 `Failed` 同等对待。
    let mut acc = Accumulator::default();
    acc.apply(assistant("m1", NodeStatus::Aborted, "被打断的答复"));
    assert_eq!(acc.final_text(), "被打断的答复");
}

// ==================== 重同步标记 ====================

/// 一帧总线信封。
fn bus_frame(kind: &str, payload: Value) -> PluginFrame {
    PluginFrame::data(serde_json::json!({
        "type": "bus_event",
        "data": { "kind": kind, "session_id": null, "data": payload }
    }))
}

#[test]
fn vdfs_resync_marker_is_recognised() {
    // 它不是一条变更（刻意不带 `path`，`event_bus_vdfs_change_of` 解不出），必须单独认——
    // 认不出就会一直等到超时。
    let frame = bus_frame(
        EVENT_BUS_KIND_VDFS,
        serde_json::json!({ "type": EVENT_BUS_RESYNC_MARKER_TYPE }),
    );
    assert!(is_resync(&frame));
}

#[test]
fn other_kinds_resync_marker_is_not_ours() {
    let frame = bus_frame(
        EVENT_BUS_KIND_SYSTEM,
        serde_json::json!({ "type": EVENT_BUS_RESYNC_MARKER_TYPE }),
    );
    assert!(!is_resync(&frame));
}

#[test]
fn a_real_change_is_not_a_resync_marker() {
    // 真变更**没有** `type: resync`——两者靠这个字段区分，顺序不可颠倒。
    let frame = bus_frame(
        EVENT_BUS_KIND_VDFS,
        serde_json::json!({ "path": SESSION_ADDR, "data": null }),
    );
    assert!(!is_resync(&frame));
}

// ==================== 会话 id 到地址 ====================

#[test]
fn session_id_is_only_a_path_segment() {
    // 地址由「根 + 挂载段 + 会话 id」拼成，会话 id 不参与路径语义——一条断言钉住
    // 这个形状，免得哪天有人把 telegram 的 chat_id 拼进更深的层级。
    let addr = format!("{}/{}/{}", "root", PLUGIN_ID_SESSION, SID);
    assert_eq!(addr, "root/session/chat-1");
}
