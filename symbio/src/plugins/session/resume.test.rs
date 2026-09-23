//! `resume` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `resume.rs` 只保留生产代码，测试全部放本文件。

use super::*;

fn tc(id: &str, status: MessageStatus) -> ChatMessage {
    ChatMessage {
        id: id.into(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::ToolCall),
        status: Some(status),
        meta: Some(json!({ "started_at": 1_700_000_000_000i64 })),
        ..Default::default()
    }
}

/// 中止收口：父 ToolCall 必须从 `Streaming` 落地为终态。
///
/// 回归：`process_tool_resume_action` 在重跑工具**之前**就把父节点广播成
/// `Streaming`（有意为之——否则用户点「批准执行」后画面毫无变化；那是广播帧，
/// 持久层拒绝瞬态落盘），而中止路径曾经直接 `return Ok(Done)`。那个节点既不是
/// 流式节点、也不会再被任何后续流程碰，于是前端**永远**显示"运行中"
/// （违反 `docs/node-state-streaming.md` §8 #11）。
#[test]
fn aborted_resume_finalizes_parent_tool_call() {
    let mut messages = vec![
        tc("tc1", MessageStatus::Streaming),
        ChatMessage {
            id: "child".into(),
            parent_id: Some("tc1".into()),
            ..Default::default()
        },
    ];

    // 返回值是**完整快照**（广播用），与就地定稿的存储副本同一形态
    let snapshot = finalize_aborted_parent(&mut messages, 0, "tc1");

    assert_eq!(
        messages[0].status,
        Some(MessageStatus::Completed),
        "父节点不得停在 Streaming：它是已落库节点，没有人会再来收口"
    );
    assert_eq!(snapshot.id, "tc1", "完整快照必须保留节点 id");
    assert_eq!(snapshot.status, Some(MessageStatus::Completed));
    assert_eq!(
        snapshot.meta.as_ref().and_then(|m| m.get("failure_kind")),
        Some(&json!("aborted")),
        "成因必须可区分：「被中止」与「批次没轮到」「被钩子拦下」不是同一件事"
    );
    assert!(
        snapshot.error.is_none(),
        "中止不是执行失败，不得在父节点挂 error —— 那会让前端多渲染一份错误文本"
    );
    assert_eq!(
        snapshot.meta.as_ref().and_then(|m| m.get("success")),
        Some(&json!(false)),
        "未完成必须如实报告 success=false（前端据此不渲染成功态）"
    );
    assert_eq!(
        snapshot.meta.as_ref().and_then(|m| m.get("started_at")),
        Some(&json!(1_700_000_000_000i64)),
        "meta 合并不得丢弃既有键（完整快照 = 权威副本 + 终态，不是只剩终态字段）"
    );
    // 子节点不受影响：收口只针对父节点
    assert_eq!(messages[1].status, None);
}

/// 索引越界即程序错误：`tc_idx` 由步骤 2 在同一份 `messages` 上定位得出，
/// 二者不可能失配——失配说明定位逻辑本身坏了，当场 panic 暴露问题
/// （而不是造一个只有 id 的假快照掩盖过去）。
#[test]
#[should_panic(expected = "步骤 2 已校验过 tc_idx 有效")]
fn aborted_resume_panics_on_out_of_range_index() {
    let mut messages: Vec<ChatMessage> = Vec::new();
    let _ = finalize_aborted_parent(&mut messages, 7, "tc-gone");
}
