//! `symbio_core/transcript_stream.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageType};
use std::sync::Arc;

/// 造一帧带唯一 `session_id` 的事件。
///
/// 订阅表是**进程级**的、用例并行跑，所以断言一律按 `session_id` 过滤：
/// 本通道会收到同进程其它用例发布的帧。
fn event_for(session_id: &str) -> NodeEvent {
    NodeEvent {
        session_id: session_id.to_string(),
        seq: 1,
        message: ChatMessage {
            id: "m1".to_string(),
            msg_type: Some(MessageType::Text),
            delta: Some("你好".to_string()),
            ..Default::default()
        },
    }
}

/// 从通道里挑出属于 `session_id` 的帧，返回**整帧**。
///
/// 容量给足 + `try_send` 同步入队 ⇒ 无需 await，`try_recv` 立刻可见。
fn take_frame(rx: &mut mpsc::Receiver<PluginFrame>, session_id: &str) -> Option<PluginFrame> {
    while let Ok(frame) = rx.try_recv() {
        let PluginFrame::Data(v) = &frame else {
            continue;
        };
        let sid = v
            .get("data")
            .and_then(|d| d.get("session_id"))
            .and_then(Value::as_str);
        if sid == Some(session_id) {
            return Some(frame);
        }
    }
    None
}

/// **扇出不复制载荷**——`PluginFrame::Data(Arc<Value>)` 存在的全部理由。
///
/// 同一帧送给 N 个订阅者时，每个订阅者拿到的是**同一份**载荷分配
/// （引用计数自增），而不是 N 份深拷贝。若有人把扇出改回「逐订阅者克隆
/// `Value`」（例如 `Arc::new((*v).clone())`），本用例立刻变红。
///
/// 用 `#[tokio::test]` 而非 `#[test]`：并行的其它用例可能把某个订阅者的通道
/// 填满，`publish_frame` 的满通道分支会 `tokio::spawn` 补送 resync 标记——
/// 同步测试里没有运行时，那一下会 panic。
#[tokio::test]
async fn fan_out_shares_one_payload_allocation() {
    let sid = format!("test-fanout-{}", uuid::Uuid::new_v4());
    let (tx1, mut rx1) = mpsc::channel(512);
    let (tx2, mut rx2) = mpsc::channel(512);
    let (c1, c2) = (format!("{sid}-a"), format!("{sid}-b"));
    register_transcript_subscriber(c1.clone(), tx1);
    register_transcript_subscriber(c2.clone(), tx2);

    publish_frame(&event_for(&sid));

    let a = take_frame(&mut rx1, &sid).expect("订阅者 A 应收到帧");
    let b = take_frame(&mut rx2, &sid).expect("订阅者 B 应收到帧");
    let (PluginFrame::Data(a), PluginFrame::Data(b)) = (a, b) else {
        panic!("转写流帧必须是 Data 帧");
    };
    assert!(
        Arc::ptr_eq(&a, &b),
        "两个订阅者必须共享同一份载荷分配；拿到两份说明扇出路径在做深拷贝"
    );

    unregister_transcript_subscriber(&c1);
    unregister_transcript_subscriber(&c2);
}

/// 线上形状 = **信封**：`{type:"transcript_event", data:{session_id, seq, message}}`。
///
/// 这条钉住的是消费端**唯一的读法**（CLI / subagent / telegram 三处共用
/// [`event_of`]）：先按信封的 `type` 分派，再从 `data` 解 `NodeEvent`。
/// 顶层直接 `from_value::<NodeEvent>` 会永远失败——`NodeEvent` 的顶层字段是
/// `session_id`/`seq`/`message`，而信封顶层是 `type`/`data`。
#[tokio::test]
async fn frame_is_an_envelope_that_decodes_back_to_the_event() {
    let sid = format!("test-envelope-{}", uuid::Uuid::new_v4());
    let (tx, mut rx) = mpsc::channel(512);
    let conn = format!("{sid}-c");
    register_transcript_subscriber(conn.clone(), tx);

    publish_frame(&event_for(&sid));

    let frame = take_frame(&mut rx, &sid).expect("应收到帧");
    let PluginFrame::Data(v) = &frame else {
        panic!("转写流帧必须是 Data 帧");
    };
    assert_eq!(v["type"], EVENT_TYPE, "判别值必须与常量同源");

    // 解包走**公共入口**（CLI / subagent / telegram 用的是同一个函数）。
    let decoded = event_of(&frame).expect("信封必须能解回 NodeEvent");
    assert_eq!(decoded.session_id, sid);
    assert_eq!(decoded.message.delta.as_deref(), Some("你好"));

    // 数据帧与背压标记互斥：同一个函数不能既解出事件又认出标记。
    assert!(!is_resync(&frame), "数据帧不是背压标记");

    unregister_transcript_subscriber(&conn);
}

/// 背压标记：`event_of` 必须返回 `None`，而 `is_resync` 必须认出来。
///
/// 两者**不可互相替代**。若消费端只看 `event_of`，标记帧会被当成「一个解不出来的
/// 事件」静默忽略——漏帧告警就此消失，正是 resync 机制要防的事。
#[test]
fn resync_marker_is_not_an_event_but_is_recognised() {
    let frame = resync_marker();
    assert!(event_of(&frame).is_none(), "背压标记无载荷，解不出事件");
    assert!(is_resync(&frame), "背压标记必须被 is_resync 认出");
}
