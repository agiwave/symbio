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

/// 顺序不变式优先：列表顺序已经错了（`existing <= cursor`）时，按数组顺序改号。
///
/// 数组顺序是权威（`replace_messages` 的契约），所以这一支仍然保留；但注意它
/// **只在顺序确实错了时**才触发——正常路径（顺序本就正确）走的是"只补缺号"。
#[test]
fn assign_seq_keeps_array_order_authoritative() {
    let mut msgs = vec![
        ChatMessage {
            id: "snapshot".into(),
            seq: None,
            ..Default::default()
        },
        ChatMessage {
            id: "keep1".into(),
            seq: Some(95),
            ..Default::default()
        },
        ChatMessage {
            id: "keep2".into(),
            seq: Some(96),
            ..Default::default()
        },
    ];
    // base = 100：压缩前会话已有的水位（本列表已有序号，故不参与填号起点）
    assign_seq(&mut msgs, 100);
    assert!(
        msgs[0].seq.unwrap() < msgs[1].seq.unwrap(),
        "seq 必须沿数组递增，否则 ordered() 会重排数组顺序"
    );
    assert!(msgs[1].seq.unwrap() < msgs[2].seq.unwrap());
    assert_eq!(msgs[1].seq, Some(95), "顺序本就正确的序号不得被改写");
}

/// 正常路径**不得**触发重排：数组顺序本就等于 seq 顺序时，既有序号原样保留。
///
/// 这条是上面那条的反面保险——修单调性不能以"每次落库都重排历史"为代价。
#[test]
fn assign_seq_leaves_already_ordered_seqs_untouched() {
    let mut msgs = vec![
        ChatMessage {
            id: "a".into(),
            seq: Some(5),
            ..Default::default()
        },
        ChatMessage {
            id: "b".into(),
            seq: Some(6),
            ..Default::default()
        },
        ChatMessage {
            id: "c".into(),
            seq: None,
            ..Default::default()
        },
    ];
    assign_seq(&mut msgs, 0);
    assert_eq!(msgs[0].seq, Some(5), "已有序的历史不得被改写");
    assert_eq!(msgs[1].seq, Some(6));
    assert_eq!(msgs[2].seq, Some(7), "缺号者续接水位");
}

/// **压缩契约**：`base` 高于既有序号时，既有序号依然一个都不许动。
///
/// 这正是实测会话 `mtmae8j2wxam4dhrei` 的病态：`replace_messages` 传
/// `base = max_seq(旧列表)`（868），而新列表是 `[快照(槽位 630), 保留区(631..642)]`。
/// 旧实现从 868 起步，把保留区整段抬到 870..881；正确行为是原样保留。
#[test]
fn assign_seq_never_rewrites_existing_seqs_even_when_base_is_higher() {
    let mut msgs = vec![
        ChatMessage {
            id: "snapshot".into(),
            // 快照接替被压缩内容的槽位：keep[0].seq - 1
            seq: Some(630),
            ..Default::default()
        },
        ChatMessage {
            id: "keep1".into(),
            seq: Some(631),
            ..Default::default()
        },
        ChatMessage {
            id: "keep2".into(),
            seq: Some(632),
            ..Default::default()
        },
    ];
    assign_seq(&mut msgs, 868);
    assert_eq!(msgs[0].seq, Some(630), "快照的槽位序号必须原样保留");
    assert_eq!(
        msgs[1].seq,
        Some(631),
        "保留区序号必须原样保留（压缩不改号）"
    );
    assert_eq!(msgs[2].seq, Some(632));
}

/// 全新列表（无任何既有序号）才使用 `base`：整批新节点接在旧水位之后。
#[test]
fn assign_seq_uses_base_only_for_fresh_lists() {
    let mut msgs = vec![
        ChatMessage {
            id: "n1".into(),
            seq: None,
            ..Default::default()
        },
        ChatMessage {
            id: "n2".into(),
            seq: None,
            ..Default::default()
        },
    ];
    assign_seq(&mut msgs, 868);
    assert_eq!(msgs[0].seq, Some(869));
    assert_eq!(msgs[1].seq, Some(870));
}

/// 开头段有**多个**无号项时，整段落在首号之前且沿数组递增。
///
/// 逐个"往上找空位"会先占用首号本身、把既有序号顶成下一个号；正确做法是
/// 整段一次算好（`首号 − k … 首号 − 1`）。
#[test]
fn assign_seq_places_leading_run_below_first_existing() {
    let mut msgs = vec![
        ChatMessage {
            id: "s1".into(),
            seq: None,
            ..Default::default()
        },
        ChatMessage {
            id: "s2".into(),
            seq: None,
            ..Default::default()
        },
        ChatMessage {
            id: "keep".into(),
            seq: Some(95),
            ..Default::default()
        },
    ];
    assign_seq(&mut msgs, 100);
    assert_eq!(msgs[0].seq, Some(93));
    assert_eq!(msgs[1].seq, Some(94));
    assert_eq!(msgs[2].seq, Some(95), "既有序号不得被开头段顶走");
}

/// 末尾段接在末号之后——本轮在途追加的常规路径。
#[test]
fn assign_seq_places_trailing_run_after_last_existing() {
    let mut msgs = vec![
        ChatMessage {
            id: "a".into(),
            seq: Some(5),
            ..Default::default()
        },
        ChatMessage {
            id: "inflight".into(),
            seq: None,
            ..Default::default()
        },
    ];
    // base 高于既有序号（= 旧列表水位）：末尾段仍须接在**末号**之后，不得跳到 base
    assign_seq(&mut msgs, 868);
    assert_eq!(msgs[0].seq, Some(5));
    assert_eq!(msgs[1].seq, Some(6));
}

/// 夹缝容不下时**顺序不变式优先**：整表按数组顺序重排（显式兜底，不静默破坏顺序）。
///
/// 该输入在真实路径上不可达（快照在开头、在途追加在末尾），但这条断言把
/// "宁可多改一次号，也不留下顺序倒挂"这个取舍钉住。
#[test]
fn assign_seq_falls_back_to_array_order_when_gap_is_full() {
    let mut msgs = vec![
        ChatMessage {
            id: "a".into(),
            seq: Some(5),
            ..Default::default()
        },
        ChatMessage {
            id: "mid".into(),
            seq: None,
            ..Default::default()
        },
        ChatMessage {
            id: "b".into(),
            seq: Some(6),
            ..Default::default()
        },
    ];
    assign_seq(&mut msgs, 0);
    assert!(
        msgs.windows(2).all(|w| w[0].seq < w[1].seq),
        "数组顺序是权威：兜底后必须严格递增，实得 {:?}",
        msgs.iter().map(|m| m.seq).collect::<Vec<_>>()
    );
}
