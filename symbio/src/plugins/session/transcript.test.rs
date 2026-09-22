//! `transcript.rs` 的单元测试（`Transcript::apply` 的图语义与线上帧格式）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{
    MessageContent, MessageRole, MessageStatus, MessageType,
};

/// 一条带正文的完整消息（`content` = 整条替换）。
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

/// 一帧增量（`delta` = 追加）。
fn delta_msg(id: &str, delta: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        delta: Some(delta.to_string()),
        ..Default::default()
    }
}

/// 删除帧。
fn removed_msg(id: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        status: Some(MessageStatus::Removed),
        ..Default::default()
    }
}

/// seq 严格单调：每个被发布的帧 +1；违例帧（同帧既带增量又带完整正文）不占号。
#[test]
fn seq_is_monotonic_and_violations_consume_no_seq() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(text_msg("a", MessageStatus::Streaming, "你"));
    tr.apply(delta_msg("a", "好"));
    // 违例：同帧既带 delta 又带 content —— 该拼接还是该替换？语义不可判定
    tr.apply(cm::ChatMessage {
        id: "a".into(),
        delta: Some("!".into()),
        content: Some(MessageContent::Text("你好!".into())),
        ..Default::default()
    });
    tr.apply(text_msg("a", MessageStatus::Completed, "你好"));
    assert_eq!(tr.seq, 3, "违例帧不得消耗 seq（3 个合法帧）");
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "你好", "content 帧是整条替换"),
        _ => panic!("应为 Text"),
    }
}

/// 两个内容字段各自的语义：`delta` 追加、`content` 整条替换——互不干扰。
#[test]
fn delta_appends_and_content_replaces() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "ab"));
    tr.apply(delta_msg("a", "cd"));
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(ref t)) => assert_eq!(t, "abcd", "delta 逐字累积"),
        _ => panic!("应为 Text"),
    }
    // 替换：把累积结果整条换掉（工具输出被改写 / 编辑消息 / 权威副本对齐都走这里）
    tr.apply(text_msg("a", MessageStatus::Completed, "XYZ"));
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "XYZ", "content 是整条替换"),
        _ => panic!("应为 Text"),
    }
}

/// 未知 id 的增量帧自给自足：就地建占位再追加，不依赖任何先行帧。
#[test]
fn delta_for_unknown_id_creates_placeholder() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(delta_msg("ghost", "片段"));
    let n = tr.get("ghost").expect("增量帧自给自足，应建占位");
    assert_eq!(n.id, "ghost");
    match n.content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "片段"),
        _ => panic!("应为 Text"),
    }
}

/// 删除帧（`status = removed`）与 persisted 的图语义。
#[test]
fn removed_and_persisted_evict() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(text_msg("a", MessageStatus::Streaming, "1"));
    tr.apply(text_msg("b", MessageStatus::Streaming, "2"));
    tr.persisted(&["a".to_string()]);
    assert!(tr.get("a").is_none(), "persisted 后节点离开在途图");
    tr.apply(removed_msg("b"));
    assert!(tr.snapshot().is_empty(), "删除帧把节点移出在途图");
    // 删除帧照常占 seq：**删了什么**在时间线上必须可追溯
    assert_eq!(tr.seq, 3);
}

/// 图里只留累积后的 `content`：`delta` 是传输形态，不得被留在节点上。
///
/// 这条断言保护的是持久层的不变量（`ensure_durable_states` 直接拒绝带 `delta`
/// 的消息）——图是在途投影的读取源，它一旦留住 `delta`，落库就会踩雷。
#[test]
fn delta_is_never_retained_on_graph_nodes() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "abc"));
    assert!(
        tr.get("a").unwrap().delta.is_none(),
        "图内节点不得携带 delta"
    );
}

/// 线上格式：帧是**一条消息**（嵌套），`delta` / `content` 各自可往返。
#[test]
fn node_event_wire_shape_is_a_message() {
    let event = NodeEvent {
        session_id: "s1".into(),
        seq: 1,
        message: delta_msg("a", "!"),
    };
    let v = serde_json::to_value(&event).unwrap();
    assert_eq!(v["session_id"], "s1");
    assert_eq!(v["seq"], 1, "外层 seq 是帧序号");
    // 嵌套而非平铺：外层帧序号与内层存储序号是两个语义不同的 seq
    assert_eq!(v["message"]["id"], "a");
    assert_eq!(v["message"]["delta"], "!");
    assert!(v.get("op").is_none(), "协议里不再有独立的操作字段");

    let back: NodeEvent = serde_json::from_value(v).unwrap();
    assert_eq!(back.seq, 1);
    assert_eq!(back.message.delta.as_deref(), Some("!"));

    // 完整消息帧：正文在 `content`，不带 `delta`
    let full = serde_json::to_value(NodeEvent {
        session_id: "s1".into(),
        seq: 2,
        message: text_msg("a", MessageStatus::Completed, "全部"),
    })
    .unwrap();
    assert!(full["message"]["delta"].is_null());
    assert_eq!(full["message"]["content"], "全部");
}

// ============================================================================
// 日志折行（`DeltaLogCoalescer`）
//
// 折行**只影响日志**：seq 分配与帧发布逐帧不变。下面这组用例一半在钉折行本身，
// 一半在钉「折行没有碰到 seq 与图」这条边界——后者才是真正的风险所在。
// ============================================================================

/// 连续同 id 的纯增量折成一行：带 seq 区间、帧数、累计字符数。
#[test]
fn consecutive_deltas_on_one_node_collapse_into_a_single_line() {
    let mut c = DeltaLogCoalescer::default();
    // seq 4/5/6 三帧，字符 8+1+2
    assert_eq!(c.feed(&delta_msg("a", "12345678"), 4), (None, true));
    assert_eq!(c.feed(&delta_msg("a", "x"), 5), (None, true));
    assert_eq!(c.feed(&delta_msg("a", "yz"), 6), (None, true));
    // 显式冲刷
    let line = c.flush().expect("待合并的 run 应被冲刷");
    assert_eq!(line, "[T#4..6] a - Update 3 帧 / +11c");
    assert_eq!(c.flush(), None, "冲刷是幂等的，不重复产出");
}

/// 换 id 立即冲刷上一段，并为新 id 开新 run。
#[test]
fn switching_node_flushes_the_previous_run() {
    let mut c = DeltaLogCoalescer::default();
    assert_eq!(c.feed(&delta_msg("a", "123"), 1), (None, true));
    assert_eq!(c.feed(&delta_msg("a", "45"), 2), (None, true));
    // 换 id：冲刷 a 的 run，b 自己开一段
    let (flushed, absorbed) = c.feed(&delta_msg("b", "6"), 3);
    assert_eq!(flushed.as_deref(), Some("[T#1..2] a - Update 2 帧 / +5c"));
    assert!(absorbed, "纯增量帧总会被并入");
    assert_eq!(c.flush().as_deref(), Some("[T#3] b - Update +1c"));
}

/// 非纯增量帧冲刷待合并的 run，且**自身不被并入**（它的行必须排在统计行之后）。
#[test]
fn a_non_delta_frame_flushes_the_run() {
    let mut c = DeltaLogCoalescer::default();
    assert_eq!(c.feed(&delta_msg("a", "12"), 1), (None, true));
    assert_eq!(c.feed(&delta_msg("a", "34"), 2), (None, true));
    // 终态帧（content、带 status）：冲刷统计行，自身不并入
    let (flushed, absorbed) = c.feed(&text_msg("a", MessageStatus::Completed, "1234"), 3);
    assert_eq!(flushed.as_deref(), Some("[T#1..2] a - Update 2 帧 / +4c"));
    assert!(!absorbed, "带状态的帧要打自己的行");
    assert_eq!(c.flush(), None, "非纯增量帧不留待合并状态");
}

/// 单帧 run 的形状与折行前**逐字相同**（只有一帧时日志形态不变）。
#[test]
fn a_single_frame_run_renders_exactly_like_before() {
    let mut c = DeltaLogCoalescer::default();
    assert_eq!(c.feed(&delta_msg("a", "abc"), 7), (None, true));
    assert_eq!(c.flush().as_deref(), Some("[T#7] a - Update +3c"));
}

/// 折行**不改 seq、不改图**：同一串帧在带折行器的转写下，seq 与节点内容必须与
/// 「逐帧发布」这一事实一致。
///
/// 这条是折行改动的真正风险边界——它把"日志优化"与"协议"分开。
#[test]
fn coalescing_does_not_touch_seq_or_the_graph() {
    let frames: Vec<cm::ChatMessage> = vec![
        text_msg("a", MessageStatus::Streaming, ""),
        delta_msg("a", "你"),
        delta_msg("a", "好"),
        delta_msg("a", "世界"),
        text_msg("a", MessageStatus::Completed, "你好世界"),
        text_msg("b", MessageStatus::Streaming, ""),
        delta_msg("b", "!"),
        removed_msg("b"),
    ];

    let mut tr = Transcript::new("s1".into());
    for f in frames {
        tr.apply(f);
    }

    // 8 帧全部发布 ⇒ seq = 8（折行没有吞掉任何一帧的号）
    assert_eq!(tr.seq, 8, "折行不得影响 seq 分配");
    assert_eq!(tr.snapshot().len(), 1, "b 已被删除，在途图只剩 a");
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(ref t)) => {
            assert_eq!(t, "你好世界", "content 帧整条替换")
        }
        _ => panic!("应为 Text"),
    }
    // 折行器在轮次收尾已被冲刷干净，不留残留状态
    assert_eq!(
        tr.delta_log.flush(),
        None,
        "apply 走完后不应残留待合并的 run"
    );
}

/// `clear` 与 `persisted` 都是冲刷点：最后一段增量不会被吞掉。
#[test]
fn clear_and_persisted_flush_the_trailing_run() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "abc"));
    tr.apply(delta_msg("a", "de"));
    // clear 之后不得残留（轮次边界）
    tr.clear();
    assert_eq!(tr.delta_log.flush(), None, "clear 应冲刷最后一段");

    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "xy"));
    tr.persisted(&["a".to_string()]);
    assert_eq!(
        tr.delta_log.flush(),
        None,
        "persisted（落库回执）应冲刷最后一段"
    );
}
