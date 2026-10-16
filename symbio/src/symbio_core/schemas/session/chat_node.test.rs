//! `symbio/src/symbio_core/schemas/session/chat_node.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// `NodeStatus::as_str()` 必须与 `serde` 的序列化名逐字一致。
///
/// 这条断言是**唯一**能拦住「VDFS 节点状态词与存储状态词分叉」的地方：
/// 两者一旦不同，前端就会按节点状态渲染出与存储不一致的角标，
/// 且编译期与运行期都不会报错（只会静默显示错的状态）。
#[test]
fn message_status_word_matches_serde() {
    for st in [
        NodeStatus::Pending,
        NodeStatus::Streaming,
        NodeStatus::WaitingUserAction,
        NodeStatus::Completed,
        NodeStatus::Aborted,
        NodeStatus::Failed,
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
    assert_eq!(NodeStatus::Completed.as_str(), "completed");
    assert_ne!(NodeStatus::Completed.as_str(), "active");
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

// ==================== [T-147c-ii-prep] 目标形状与 style_tag ====================

/// 六值 wire 词都能被 [`legacy_shape_to_style_tag`] 折成目标五值 + 对应 style_tag。
/// 这张映射表是 [T-147c-ii] 落地的**判据基线**：`NodeKind` 收五值那一档，读侧兼容代码
/// 就靠这条；测试**先立住映射**，五值改完之后再回读一次。
#[test]
fn legacy_wire_words_fold_to_shape_and_style_tag() {
    for (word, shape, tag) in [
        ("text", NodeShape::Body, None),
        ("reasoning", NodeShape::Body, Some(CHAT_STYLE_TAG_REASONING)),
        ("turn", NodeShape::Skeleton, Some(CHAT_STYLE_TAG_RUN)),
        ("tool_call", NodeShape::Skeleton, Some(CHAT_STYLE_TAG_TOOL)),
        ("user_prompt", NodeShape::Interaction, None),
        ("compression", NodeShape::Notice, Some(CHAT_STYLE_TAG_COMPRESSION)),
        // 新词也认（未来 wire 上的读侧）
        ("body", NodeShape::Body, None),
        ("skeleton", NodeShape::Skeleton, None),
        ("notice", NodeShape::Notice, None),
        ("interaction", NodeShape::Interaction, None),
        ("typing", NodeShape::Typing, None),
    ] {
        let (got_shape, got_tag) = legacy_shape_to_style_tag(word)
            .unwrap_or_else(|| panic!("{word} 未登记进映射"));
        assert_eq!(got_shape, shape, "{word} 折错形状");
        assert_eq!(got_tag, tag, "{word} 折错 style_tag");
    }
    // 未登记词 ⇒ None（不假装知道）。
    assert!(legacy_shape_to_style_tag("nonsense").is_none());
}

/// 谓词按**当前六值形态**判，且**互斥**——`is_reasoning` 与 `is_text` 不能同真（曾经的
/// bug 是 Text 与 Reasoning 都判成 Body）。[T-147c-ii] 五值收敛那一档 `is_*` 谓词**签名不变**，
/// 实现切到 `shape() + style_tag()`；这一档的断言是"迁移后消费方不用改"的护栏。
#[test]
fn predicates_are_exclusive_per_kind() {
    let n = |kind: NodeKind| ChatNode {
        msg_type: Some(kind),
        ..Default::default()
    };
    let run = n(NodeKind::Turn);
    assert!(run.is_run());
    assert!(!run.is_tool_call());
    assert!(!run.is_reasoning());
    assert!(!run.is_text());

    let tc = n(NodeKind::ToolCall);
    assert!(tc.is_tool_call());
    assert!(!tc.is_run());

    let reasoning = n(NodeKind::Reasoning);
    assert!(reasoning.is_reasoning());
    assert!(!reasoning.is_text(), "推理与正文互斥（曾同时命中 Body 是 bug）");

    let text = n(NodeKind::Text);
    assert!(text.is_text());
    assert!(!text.is_reasoning());

    let comp = n(NodeKind::Compression);
    assert!(comp.is_compression());
    assert!(!comp.is_interaction());

    let prompt = n(NodeKind::UserPrompt);
    assert!(prompt.is_interaction());

    let bare = ChatNode::default();
    assert!(
        !bare.is_text() && !bare.is_run() && !bare.is_tool_call() && !bare.is_reasoning(),
        "未标 msg_type ⇒ 全谓词都假（不是默认落到 Text）"
    );
}

/// `shape()` 与 `style_tag()` 是 [T-147c-ii] 落地后消费方的**判据入口**——本批先把
/// 六值映射立住，五值收敛那一档 `msg_type` 变成五值，`shape()` 直接返回 `msg_type` 对应。
#[test]
fn shape_helper_maps_six_variants_to_five() {
    let n = |kind: NodeKind| ChatNode {
        msg_type: Some(kind),
        ..Default::default()
    };
    assert_eq!(n(NodeKind::Text).shape(), Some(NodeShape::Body));
    assert_eq!(n(NodeKind::Reasoning).shape(), Some(NodeShape::Body));
    assert_eq!(n(NodeKind::Turn).shape(), Some(NodeShape::Skeleton));
    assert_eq!(n(NodeKind::ToolCall).shape(), Some(NodeShape::Skeleton));
    assert_eq!(n(NodeKind::UserPrompt).shape(), Some(NodeShape::Interaction));
    assert_eq!(n(NodeKind::Compression).shape(), Some(NodeShape::Notice));
    assert_eq!(ChatNode::default().shape(), None);
}

/// `style_tag()` 从 `meta.style_tag` 读——**新形态**的写方（[T-147c-ii] 之后）用它带出
/// 同 shape 下的呈现变体。本批读侧兼容代码已就位，五值那一档只补构造侧。
#[test]
fn style_tag_reads_from_meta() {
    let mut n = ChatNode::default();
    assert_eq!(n.style_tag(), None);
    n.meta = Some(serde_json::json!({ "style_tag": CHAT_STYLE_TAG_RUN }));
    assert_eq!(n.style_tag(), Some(CHAT_STYLE_TAG_RUN));
}
