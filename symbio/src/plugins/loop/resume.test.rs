//! `resume` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `resume.rs` 只保留生产代码，测试全部放本文件。

use super::*;

fn tc(id: &str, status: NodeStatus) -> ChatNode {
    ChatNode {
        id: id.into(),
        role: Some(NodeRole::Assistant),
        msg_type: Some(NodeKind::Skeleton),
        status: Some(status),
        meta: Some(json!({ "style_tag": "tool", "started_at": 1_700_000_000_000i64 })),
        ..Default::default()
    }
}

/// 中止收口：父 ToolCall 必须从 `Streaming` 落地为终态。
///
/// 回归：`process_tool_resume_action` 在重跑工具**之前**就把父节点广播成
/// `Streaming`（有意为之——否则用户点「批准执行」后画面毫无变化；那是广播帧，
/// 持久层拒绝瞬态落盘），而中止路径不得直接 `return Ok(Done)`。那个节点既不是
/// 流式节点、也不会再被任何后续流程碰，于是前端**永远**显示"运行中"
/// （违反 `symbio/src/plugins/session/docs/018-node-state-streaming.md` §8 #11）。
#[test]
fn aborted_resume_finalizes_parent_tool_call() {
    let mut messages = vec![
        tc("tc1", NodeStatus::Streaming),
        ChatNode {
            id: "child".into(),
            parent_id: Some("tc1".into()),
            ..Default::default()
        },
    ];

    // 返回值是**完整快照**（广播用），与就地定稿的存储副本同一形态
    let snapshot = finalize_aborted_parent(&mut messages, 0, "tc1");

    assert_eq!(
        messages[0].status,
        Some(NodeStatus::Completed),
        "父节点不得停在 Streaming：它是已落库节点，没有人会再来收口"
    );
    assert_eq!(snapshot.id, "tc1", "完整快照必须保留节点 id");
    assert_eq!(snapshot.status, Some(NodeStatus::Completed));
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
    let mut messages: Vec<ChatNode> = Vec::new();
    let _ = finalize_aborted_parent(&mut messages, 7, "tc-gone");
}

// ── 恢复路的列表变更：经动作面交给会话 ──
//
// 删除-重建模式里，「哪些节点没了」由会话的动作面落库并发帧（ADR-086 §4：写动作
// 自己发帧）。驱动器只负责**算出整棵子树**并把它交过去——算漏一个节点，重建后
// 子树复活；把用户格算进去，重试就把本轮发言也删了。事实落格本身归会话
// （`message_events::removed`），由 session 的用例钉住；这里钉的是**交给谁、交了
// 什么**。
//
// 反向自检：把 BFS 里的 `turn_root_of` 并集去掉 ⇒ 扁平档下升根的子孙漏删，第一
// 条用例红。

use std::sync::Arc;

use super::super::test_support::{default_settings, turn_spec, FakeSessionPort};
use super::super::turn_session::LoopSession;

/// 一份「失败轮」转写：用户格 + Failed Turn + 半截子节点。
fn failed_turn_messages() -> Vec<ChatNode> {
    vec![
        ChatNode {
            id: "u-1".into(),
            role: Some(NodeRole::User),
            msg_type: Some(NodeKind::Body),
            content: Some(MessageContent::Text("第一问".into())),
            status: Some(NodeStatus::Completed),
            seq: Some(1),
            ..Default::default()
        },
        ChatNode {
            id: "t-failed".into(),
            role: Some(NodeRole::Assistant),
            msg_type: Some(NodeKind::Skeleton),
            meta: Some(serde_json::json!({"style_tag": "run"})),
            status: Some(NodeStatus::Failed),
            seq: Some(2),
            ..Default::default()
        },
        ChatNode {
            id: "n-1".into(),
            parent_id: Some("t-failed".into()),
            role: Some(NodeRole::Assistant),
            msg_type: Some(NodeKind::Body),
            content: Some(MessageContent::Text("半截".into())),
            status: Some(NodeStatus::Completed),
            seq: Some(3),
            ..Default::default()
        },
    ]
}

/// 装配：驱动器的会话句柄 + 假动作面（消息列表在内存里）。
fn setup_with_failed_turn() -> (LoopSession, Arc<FakeSessionPort>) {
    let port = Arc::new(FakeSessionPort::new(failed_turn_messages()));
    let spec = turn_spec(
        "resume-test",
        std::env::temp_dir().join("symbio-test/resume-tool-archives"),
        std::env::temp_dir().join("symbio-test/resume-transcripts"),
        default_settings(),
    );
    (LoopSession::new(spec, None, port.clone()), port)
}

/// **重试失败轮**：整棵子树一次性交给会话动作面，用户格不在其中。
#[tokio::test]
async fn retry_subtree_hands_the_whole_subtree_to_the_session_action() {
    let (session, port) = setup_with_failed_turn();
    let req = ResumeRequest {
        target_id: "t-failed".into(),
        action: ResumeAction::RetrySubtree,
        args: None,
        reason: None,
        answer: None,
    };

    super::process_retry_subtree(&session, &req)
        .await
        .expect("重试失败轮应成功");

    let calls = port.removed.lock().unwrap();
    assert_eq!(calls.len(), 1, "一次重试只发一次删除动作");
    let mut ids = calls[0].clone();
    ids.sort();
    assert_eq!(
        ids,
        vec!["n-1".to_string(), "t-failed".to_string()],
        "Turn 自身 + 它的子节点，一个不漏"
    );
    // 用户格**不在**删除列表里：重试重跑的是同一轮，用户那句话还在历史中
    // （`process_retry_subtree` 的注释与 chat_loop 的续写都依赖这一点）。
    assert!(
        !calls[0].contains(&"u-1".to_string()),
        "用户发言不该被删：{:?}",
        calls[0]
    );
}

/// **目标不是 Turn** ⇒ 报 `NotFound`，且**一条删除动作都不发**。
///
/// 「什么都没删」不该在动作面上留下痕迹——与 `truncate_messages` 空列表不落格、
/// `message_events::removed` 自判空列表同一条判据。
#[tokio::test]
async fn retry_subtree_on_a_missing_target_lands_nothing() {
    let (session, port) = setup_with_failed_turn();
    let req = ResumeRequest {
        target_id: "n-1".into(), // 存在，但不是 Turn 节点
        action: ResumeAction::RetrySubtree,
        args: None,
        reason: None,
        answer: None,
    };

    // `ResumeOutcome` 不带 `Debug`（它是**结果**不是诊断载体）⇒ 不用 `expect_err`，
    // 直接解 `Result`。
    let err = match super::process_retry_subtree(&session, &req).await {
        Ok(_) => panic!("目标不是 Turn ⇒ 应报 NotFound，实际却成功了"),
        Err(e) => e,
    };
    assert!(
        matches!(err, crate::symbio_core::PluginError::NotFound(_)),
        "应是 NotFound（实得 {err:?}）"
    );

    assert!(
        port.removed.lock().unwrap().is_empty(),
        "拒绝路径不得发任何删除动作"
    );
}

// ── 恢复上下文的取值：不伪造默认工具名 ──────────────────────────────────

/// 交互子节点带 `meta.prompt.tool_name` ⇒ 用它。
#[test]
fn extract_uses_prompt_tool_name_when_present() {
    let mut parent = tc("tc1", NodeStatus::Completed);
    parent.meta = Some(json!({ "tool_name": "write_file" }));
    let child = ChatNode {
        id: "child".into(),
        parent_id: Some("tc1".into()),
        msg_type: Some(NodeKind::Interaction),
        meta: Some(json!({
            "prompt": { "tool_name": "ask_user", "args": { "q": "继续吗" } }
        })),
        ..Default::default()
    };
    let messages = vec![parent, child];
    let (name, args) = extract_tool_context(&messages, 0, 1);
    assert_eq!(name, "ask_user", "prompt 里写着哪一个，就恢复哪一个");
    assert_eq!(args["q"], "继续吗");
}

/// 交互子节点**没有** `prompt.tool_name` ⇒ 落到父节点，不伪造 `ask_user`。
///
/// 这是本组唯一有回归价值的一条：此前的实现 `unwrap_or("ask_user")` 把
/// `local` 插件的工具名写进了 `loop`——而 `loop` 不拥有那份知识。真发生时的
/// 表现是「用错的工具去恢复」（会真的执行一次 `ask_user`），比「取不到上下文」
/// 难查得多。
#[test]
fn extract_falls_back_to_parent_instead_of_faking_a_tool_name() {
    let mut parent = tc("tc1", NodeStatus::Streaming);
    parent.meta = Some(json!({ "tool_name": "write_file", "args": { "path": "a.txt" } }));
    let child = ChatNode {
        id: "child".into(),
        parent_id: Some("tc1".into()),
        msg_type: Some(NodeKind::Interaction),
        // 有 prompt，但没有 tool_name
        meta: Some(json!({ "prompt": { "message": "批准执行吗" } })),
        ..Default::default()
    };
    let messages = vec![parent, child];
    let (name, args) = extract_tool_context(&messages, 0, 1);
    assert_eq!(
        name, "write_file",
        "取不到 prompt.tool_name 时应回退到父 ToolCall，而不是默认成 ask_user"
    );
    assert_eq!(args["path"], "a.txt");
}
