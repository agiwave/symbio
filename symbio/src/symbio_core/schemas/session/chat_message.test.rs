//! `symbio/src/symbio_core/schemas/session/chat_message.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// `MessageStatus::as_str()` 必须与 `serde` 的序列化名逐字一致。
///
/// 这条断言是**唯一**能拦住「VDFS 节点状态词与存储状态词分叉」的地方：
/// 两者一旦不同，前端就会按节点状态渲染出与存储不一致的角标，
/// 且编译期与运行期都不会报错（只会静默显示错的状态）。
#[test]
fn message_status_word_matches_serde() {
    for st in [
        MessageStatus::Pending,
        MessageStatus::Streaming,
        MessageStatus::WaitingUserAction,
        MessageStatus::Completed,
        MessageStatus::Aborted,
        MessageStatus::Failed,
    ] {
        let wire = serde_json::to_value(&st).unwrap();
        assert_eq!(
            wire.as_str(),
            Some(st.as_str()),
            "状态词与序列化名分叉：{st:?}"
        );
    }
}

/// `Completed` 不得与「未标注」被折叠成同一个节点状态词。
///
/// 折叠曾让消费端必须把 `active` **猜回** `completed`（一次信息丢失 + 一次还原）；
/// 这条断言把"两者可区分"钉住。
#[test]
fn completed_is_distinct_from_unset_sentinel() {
    assert_eq!(MessageStatus::Completed.as_str(), "completed");
    assert_ne!(MessageStatus::Completed.as_str(), "active");
}

/// `ResumeAction` 的**线格式词**是跨栈契约：前端 `ResumePayload.action` 的
/// 字面量必须与它逐字相等。
///
/// 与 `message_status_word_matches_serde` 同一动机——两侧分叉时编译期与运行期
/// 都不报错，只会在运行期表现为「点了重试没反应」（后端 serde 反序列化失败）。
/// 因此把全部取值都钉住，而不只是本次新增的那个。
#[test]
fn resume_action_wire_words_are_snake_case() {
    let cases = [
        (ResumeAction::RetryTurn, "retry_turn"),
        (ResumeAction::Retry, "retry"),
        (ResumeAction::Approve, "approve"),
        (ResumeAction::Reject, "reject"),
        (ResumeAction::Supply, "supply"),
        (ResumeAction::Answer, "answer"),
        (ResumeAction::RetryCompaction, "retry_compaction"),
    ];
    for (action, word) in cases {
        assert_eq!(
            serde_json::to_value(&action).unwrap().as_str(),
            Some(word),
            "线格式词与前端字面量分叉：{action:?}"
        );
        // 反向：前端发来的字符串必须能解回同一个取值
        let back: ResumeAction = serde_json::from_value(serde_json::json!(word)).unwrap();
        assert_eq!(format!("{back:?}"), format!("{action:?}"));
    }
}
