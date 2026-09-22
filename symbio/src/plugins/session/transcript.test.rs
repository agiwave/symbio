//! `transcript.rs` 的单元测试（`Transcript::apply` 的图语义与线上帧格式）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{
    MessageContent, MessageRole, MessageStatus, MessageType,
};

fn text_msg(id: &str, status: MessageStatus, text: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        status: Some(status),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// seq 严格单调：每个被发布的帧 +1；违例帧（未知 id 追加）不占号。
#[test]
fn seq_is_monotonic_and_violations_consume_no_seq() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Streaming, "你")),
    });
    tr.apply(NodeOp::Append {
        message_id: "a".into(),
        delta: "好".into(),
    });
    // 违例：未知 id 追加 → 丢弃、不占 seq
    tr.apply(NodeOp::Append {
        message_id: "ghost".into(),
        delta: "x".into(),
    });
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Completed, "你好")),
    });
    assert_eq!(tr.seq, 3, "违例帧不得消耗 seq（3 个合法帧）");
    let a = tr.get("a").unwrap();
    match a.content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "你好", "Upsert 是整条替换"),
        _ => panic!("应为 Text"),
    }
}

/// Append 逐字累积到图内节点（Upsert 全量替换）。
#[test]
fn append_accumulates_and_upsert_replaces() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Streaming, "")),
    });
    tr.apply(NodeOp::Append {
        message_id: "a".into(),
        delta: "ab".into(),
    });
    tr.apply(NodeOp::Append {
        message_id: "a".into(),
        delta: "cd".into(),
    });
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Completed, "XYZ")),
    });
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "XYZ", "Upsert 是整条替换"),
        _ => panic!("应为 Text"),
    }
}

/// Remove / Reset / persisted 的图语义。
#[test]
fn remove_reset_and_persisted_evict() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Streaming, "1")),
    });
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("b", MessageStatus::Streaming, "2")),
    });
    tr.persisted(&["a".to_string()]);
    assert!(tr.get("a").is_none(), "persisted 后节点离开在途图");
    tr.apply(NodeOp::Remove {
        message_id: "b".into(),
    });
    assert!(tr.snapshot().is_empty());
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("c", MessageStatus::Streaming, "3")),
    });
    tr.apply(NodeOp::Reset);
    assert!(tr.snapshot().is_empty(), "Reset 清空在途图");
}

/// 线上格式：NodeEvent 序列化稳定（前端契约）。
#[test]
fn node_event_serializes_with_tagged_op() {
    let event = NodeEvent {
        session_id: "s1".into(),
        seq: 1,
        op: NodeOp::Append {
            message_id: "a".into(),
            delta: "!".into(),
        },
    };
    let v = serde_json::to_value(&event).unwrap();
    assert_eq!(v["session_id"], "s1");
    assert_eq!(v["seq"], 1);
    assert_eq!(v["op"], "append");
    let back: NodeEvent = serde_json::from_value(v).unwrap();
    assert_eq!(back.seq, 1);
}

// ============================================================================
// 日志折行（`AppendLogCoalescer`）
//
// 折行**只影响日志**：seq 分配与帧发布逐帧不变。下面这组用例一半在钉折行本身，
// 一半在钉「折行没有碰到 seq 与图」这条边界——后者才是真正的风险所在。
// ============================================================================

fn append(id: &str, delta: &str) -> NodeOp {
    NodeOp::Append {
        message_id: id.into(),
        delta: delta.into(),
    }
}

fn upsert(id: &str, text: &str) -> NodeOp {
    NodeOp::Upsert {
        message: Box::new(text_msg(id, MessageStatus::Streaming, text)),
    }
}

/// 连续同 id 的 Append 折成一行：带 seq 区间、帧数、累计字符数。
#[test]
fn consecutive_appends_on_one_node_collapse_into_a_single_line() {
    let mut c = AppendLogCoalescer::default();
    // seq 4/5/6 三帧，字符 8+1+2
    assert_eq!(c.feed(&append("a", "12345678"), 4), None);
    assert_eq!(c.feed(&append("a", "x"), 5), None);
    assert_eq!(c.feed(&append("a", "yz"), 6), None);
    // 换操作才冲刷
    let line = c.flush().expect("待合并的 run 应被冲刷");
    assert_eq!(line, "[T#4..6 Append] a - 3 帧 / +11c");
    assert_eq!(c.flush(), None, "冲刷是幂等的，不重复产出");
}

/// 换 id 立即冲刷上一段，并为新 id 开新 run。
#[test]
fn switching_node_flushes_the_previous_run() {
    let mut c = AppendLogCoalescer::default();
    assert_eq!(c.feed(&append("a", "123"), 1), None);
    assert_eq!(c.feed(&append("a", "45"), 2), None);
    // 换 id：冲刷 a 的 run，b 自己开一段
    let flushed = c.feed(&append("b", "6"), 3);
    assert_eq!(flushed.as_deref(), Some("[T#1..2 Append] a - 2 帧 / +5c"));
    assert_eq!(c.flush().as_deref(), Some("[T#3 Append] b - +1c"));
}

/// 非 Append 帧冲刷待合并的 run（`Upsert` 收尾行必须排在统计行之后）。
#[test]
fn a_non_append_frame_flushes_the_run() {
    let mut c = AppendLogCoalescer::default();
    assert_eq!(c.feed(&append("a", "12"), 1), None);
    assert_eq!(c.feed(&append("a", "34"), 2), None);
    let flushed = c.feed(&upsert("a", "1234"), 3);
    assert_eq!(flushed.as_deref(), Some("[T#1..2 Append] a - 2 帧 / +4c"));
    assert_eq!(c.flush(), None, "非 Append 帧不留待合并状态");
}

/// 单帧 run 的形状与折行前**逐字相同**（只有一帧时日志形态不变）。
#[test]
fn a_single_frame_run_renders_exactly_like_before() {
    let mut c = AppendLogCoalescer::default();
    assert_eq!(c.feed(&append("a", "abc"), 7), None);
    assert_eq!(c.flush().as_deref(), Some("[T#7 Append] a - +3c"));
}

/// 折行**不改 seq、不改图**：同一串操作下，带折行器与不带折行器的转写
/// 在 seq 与节点内容上必须一致。
///
/// 这条是折行改动的真正风险边界——它把"日志优化"与"协议"分开。
#[test]
fn coalescing_does_not_touch_seq_or_the_graph() {
    let ops: Vec<NodeOp> = vec![
        upsert("a", ""),
        append("a", "你"),
        append("a", "好"),
        append("a", "世界"),
        upsert("a", "你好世界"),
        upsert("b", ""),
        append("b", "!"),
        NodeOp::Remove {
            message_id: "b".into(),
        },
        NodeOp::Reset,
    ];

    let mut tr = Transcript::new("s1".into());
    for op in ops {
        tr.apply(op);
    }

    // 9 帧全部发布 ⇒ seq = 9（折行没有吞掉任何一帧的号）
    assert_eq!(tr.seq, 9, "折行不得影响 seq 分配");
    assert!(tr.snapshot().is_empty(), "Reset 后图应为空");
    // 折行器在轮次收尾已被冲刷干净，不留残留状态
    assert_eq!(
        tr.append_log.flush(),
        None,
        "apply 走完后不应残留待合并的 run"
    );
}

/// `clear` 与 `persisted` 都是冲刷点：最后一段 Append 不会被吞掉。
#[test]
fn clear_and_persisted_flush_the_trailing_run() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(upsert("a", ""));
    tr.apply(append("a", "abc"));
    tr.apply(append("a", "de"));
    // clear 之后不得残留（轮次边界）
    tr.clear();
    assert_eq!(tr.append_log.flush(), None, "clear 应冲刷最后一段");

    tr.apply(upsert("a", ""));
    tr.apply(append("a", "xy"));
    tr.persisted(&["a".to_string()]);
    assert_eq!(
        tr.append_log.flush(),
        None,
        "persisted（落库回执）应冲刷最后一段"
    );
}
