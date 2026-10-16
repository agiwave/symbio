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
        (ResumeAction::RetrySubtree, "retry_subtree"),
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

// ==================== [T-147c-ii] 目标形状与 style_tag ====================

/// 六词旧 wire 词 + 五词新 wire 词都能被 [`legacy_shape_to_style_tag`] 折成
/// 目标五值 + 对应 style_tag。这张映射表是 [`message_of_node`] 读边界折叠存量
/// VDFS 属性的**判据基线**：旧六值已删出 `NodeKind`，旧词只活在这条读侧路径里。
#[test]
fn legacy_wire_words_fold_to_shape_and_style_tag() {
    for (word, shape, tag) in [
        ("text", NodeKind::Body, None),
        ("reasoning", NodeKind::Body, Some(CHAT_STYLE_TAG_REASONING)),
        ("turn", NodeKind::Skeleton, Some(CHAT_STYLE_TAG_RUN)),
        ("tool_call", NodeKind::Skeleton, Some(CHAT_STYLE_TAG_TOOL)),
        ("user_prompt", NodeKind::Interaction, None),
        (
            "compression",
            NodeKind::Notice,
            Some(CHAT_STYLE_TAG_COMPRESSION),
        ),
        // 新词——原样，无 tag 注入
        ("body", NodeKind::Body, None),
        ("skeleton", NodeKind::Skeleton, None),
        ("notice", NodeKind::Notice, None),
        ("interaction", NodeKind::Interaction, None),
        ("typing", NodeKind::Typing, None),
    ] {
        let (got_shape, got_tag) =
            legacy_shape_to_style_tag(word).unwrap_or_else(|| panic!("{word} 未登记进映射"));
        assert_eq!(got_shape, shape, "{word} 折错形状");
        assert_eq!(got_tag, tag, "{word} 折错 style_tag");
    }
    // 未登记词 ⇒ None（不假装知道）。
    assert!(legacy_shape_to_style_tag("nonsense").is_none());
}

/// 谓词按「目标变体 + `meta.style_tag`」判，且**互斥**——`is_reasoning` 与
/// `is_text` 不能同真；Skeleton 的变体判别全靠 tag，漏 tag 不会被静默归队。
#[test]
fn predicates_are_exclusive_per_form() {
    let n = |kind: NodeKind| ChatNode {
        msg_type: Some(kind),
        ..Default::default()
    };

    let run = n(NodeKind::Skeleton).with_style_tag(CHAT_STYLE_TAG_RUN);
    assert!(run.is_run());
    assert!(!run.is_tool_call());
    assert!(!run.is_reasoning());
    assert!(!run.is_text());

    let tc = n(NodeKind::Skeleton).with_style_tag(CHAT_STYLE_TAG_TOOL);
    assert!(tc.is_tool_call());
    assert!(!tc.is_run());

    let reasoning = n(NodeKind::Body).with_style_tag(CHAT_STYLE_TAG_REASONING);
    assert!(reasoning.is_reasoning());
    assert!(
        !reasoning.is_text(),
        "推理与正文互斥（同落 Body 后靠 tag 区分）"
    );

    let text = n(NodeKind::Body);
    assert!(text.is_text());
    assert!(!text.is_reasoning());

    assert!(n(NodeKind::Notice).is_compression());
    assert!(n(NodeKind::Interaction).is_interaction());

    // 裸 Skeleton（无 tag）：两个骨架谓词都不真——写方漏 tag 不会被静默归队
    let bare_skel = n(NodeKind::Skeleton);
    assert!(!bare_skel.is_run() && !bare_skel.is_tool_call());

    let bare = ChatNode::default();
    assert!(
        !bare.is_text() && !bare.is_run() && !bare.is_tool_call() && !bare.is_reasoning(),
        "未标 msg_type ⇒ 全谓词都假（不是默认落到 Body）"
    );
}

/// `style_tag()` 从 `meta.style_tag` 读；`with_style_tag` 对已有 meta 对象是**并入**
/// 而不是整颗替换。
#[test]
fn style_tag_reads_and_merges_meta() {
    let mut n = ChatNode::default();
    assert_eq!(n.style_tag(), None);
    n.meta = Some(serde_json::json!({ "style_tag": CHAT_STYLE_TAG_RUN }));
    assert_eq!(n.style_tag(), Some(CHAT_STYLE_TAG_RUN));

    let with_meta = ChatNode {
        msg_type: Some(NodeKind::Skeleton),
        meta: Some(serde_json::json!({ "prompt": { "tool_name": "x" } })),
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_TOOL);
    let meta = with_meta.meta.clone().unwrap();
    assert_eq!(with_meta.style_tag(), Some(CHAT_STYLE_TAG_TOOL));
    assert!(meta.get("prompt").is_some(), "既有 meta 键不得被挤掉");
}

// ==================== [T-147c-ii-s4] message_of_node 读边界折叠存量旧词 ====================

/// 用 `attributes` JSON 造一个 VDFS 节点（`message_of_node` 的输入形状）。
fn vdfs_with_attrs(name: &str, attrs: serde_json::Value) -> crate::symbio_core::VdfsNode {
    crate::symbio_core::VdfsNode {
        name: name.to_string(),
        status: "completed".to_string(),
        attributes: attrs.as_object().cloned().unwrap_or_default(),
        ..Default::default()
    }
}

/// 旧六词的历史数据（VDFS `attributes.type`）经 `message_of_node` 折成新形态 +
/// 注入 `style_tag`——谓词在折叠结果上必须命中，且既有 meta 键不丢。
#[test]
fn message_of_node_folds_legacy_words() {
    let m = message_of_node(
        &vdfs_with_attrs("a", serde_json::json!({"type": "turn"})),
        String::new(),
    )
    .unwrap();
    assert!(m.is_run(), "旧词 turn 必须折成 run 骨架");

    let m = message_of_node(
        &vdfs_with_attrs(
            "b",
            serde_json::json!({"type": "compression", "meta": {"tokens_before": 100}}),
        ),
        String::new(),
    )
    .unwrap();
    assert!(m.is_compression());
    assert_eq!(
        m.style_tag(),
        Some(CHAT_STYLE_TAG_COMPRESSION),
        "旧词 compression 折出 Notice 后必须带 tag"
    );
    assert_eq!(
        m.meta.as_ref().unwrap().get("tokens_before").unwrap(),
        &serde_json::json!(100),
        "注入 style_tag 不得挤掉既有 meta 键"
    );

    // meta 为 null（`message_node` 的「没有 meta」写法）时注入要造出对象
    let m = message_of_node(
        &vdfs_with_attrs("c", serde_json::json!({"type": "reasoning", "meta": null})),
        "思考".to_string(),
    )
    .unwrap();
    assert!(m.is_reasoning());
}

/// 新词的读侧：serde 词经 `legacy_shape_to_style_tag` 原样通过；meta 里已有的
/// `style_tag` 不被旧折叠覆盖；未登记词与缺词都是 `msg_type = None`。
#[test]
fn message_of_node_keeps_target_words_and_existing_tags() {
    let m = message_of_node(
        &vdfs_with_attrs(
            "d",
            serde_json::json!({"type": "skeleton", "meta": {"style_tag": "tool"}}),
        ),
        String::new(),
    )
    .unwrap();
    assert!(m.is_tool_call());

    // 异常共存（不该发生，钉住「不覆盖」规则本身）：tag 已存在 ⇒ 保留 meta 里的值
    let m = message_of_node(
        &vdfs_with_attrs(
            "e",
            serde_json::json!({"type": "body", "meta": {"style_tag": "reasoning"}}),
        ),
        String::new(),
    )
    .unwrap();
    assert!(m.is_reasoning());

    let m = message_of_node(
        &vdfs_with_attrs("f", serde_json::json!({"type": "nonsense"})),
        String::new(),
    )
    .unwrap();
    assert_eq!(m.msg_type, None, "未登记词不假装知道");

    let m = message_of_node(&vdfs_with_attrs("g", serde_json::json!({})), String::new()).unwrap();
    assert_eq!(m.msg_type, None);
}
