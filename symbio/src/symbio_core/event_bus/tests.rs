//! `symbio_core/event_bus.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
use std::sync::Arc;

/// 串行化本文件的用例。
///
/// `SUBSCRIBERS` 是**进程级**的静态表，而 `cargo test` 默认并行跑用例：
/// 每个用例注册自己的订阅者后，**其它用例的每一次 `try_publish` 也会投进它的通道**。
/// 对「容量 1 + 依赖 Full 触发 resync」的用例，这会把时序搅成非确定——
/// 实测：加了 `fan_out_shares_one_payload_allocation`（它自己也要发布）之后，
/// `full_channel_eventually_receives_resync_marker` 在并行下会随机超时，串行则必过。
///
/// 锁**只覆盖本文件**：这里是用例间唯一的耦合面（同一张全局表）。
/// 用 `tokio::sync::Mutex` 而不是 `std::sync::Mutex`——守卫要跨 `.await` 持有。
static SERIAL: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

/// **满通道不得摘除订阅**——本模块最重要的一条不变量。
///
/// 摘除是**不可逆**的：前端不会收到任何信号（Tauri 连接仍活着，不触发
/// `disconnected`），此后永久收不到 VDFS 变更。而丢一帧只是「少一次状态更新」。
#[tokio::test]
async fn full_channel_keeps_subscription() {
    let _g = SERIAL.lock().await;
    let (tx, mut rx) = mpsc::channel(1);
    let id = "test_full_keeps_subscription".to_string();
    event_bus_register_subscriber(id.clone(), tx);

    EventBus::try_publish(EVENT_BUS_KIND_VDFS, None, json!({ "n": 1 })); // 填满容量 1
    EventBus::try_publish(EVENT_BUS_KIND_VDFS, None, json!({ "n": 2 })); // Full

    assert!(
        SUBSCRIBERS.contains_key(&id),
        "通道满只允许丢帧，不得摘除订阅"
    );
    assert!(rx.try_recv().is_ok(), "满之前的帧应已送达");
    event_bus_unregister_subscriber(&id);
}

/// 对端断开（`Closed`）才摘除——消费者走了，留着只泄漏。
#[tokio::test]
async fn closed_channel_is_unregistered() {
    let _g = SERIAL.lock().await;
    let (tx, rx) = mpsc::channel(4);
    let id = "test_closed_is_unregistered".to_string();
    event_bus_register_subscriber(id.clone(), tx);
    drop(rx);

    EventBus::try_publish(EVENT_BUS_KIND_VDFS, None, json!({}));

    assert!(
        !SUBSCRIBERS.contains_key(&id),
        "对端断开必须摘除，否则订阅表泄漏"
    );
}

/// resync 标记的形状：是 `bus_event`，且**不带 `path`**。
///
/// 不带 `path` 是有意的——既有消费者的作用域判定要求 `path` 是字符串，
/// 因此新指令不会污染它们已有的输入。
#[test]
fn resync_marker_is_a_bus_event_without_path() {
    let PluginFrame::Data(v) = resync_marker(EVENT_BUS_KIND_VDFS) else {
        panic!("resync 标记必须是 Data 帧");
    };
    assert_eq!(v["type"], "bus_event");
    assert_eq!(v["data"]["kind"], EVENT_BUS_KIND_VDFS);
    assert_eq!(v["data"]["data"]["type"], EVENT_BUS_RESYNC_MARKER_TYPE);
    assert!(
        v["data"]["data"].get("path").is_none(),
        "resync 是指令而非变更，不得带 path（否则会被当成一条变更消费）"
    );
}

/// 满通道后，消费端应**实际收到** resync 标记（腾出空间后补送成功）。
#[tokio::test]
async fn full_channel_eventually_receives_resync_marker() {
    let _g = SERIAL.lock().await;
    let (tx, mut rx) = mpsc::channel(1);
    let id = "test_resync_delivered".to_string();
    event_bus_register_subscriber(id.clone(), tx);

    EventBus::try_publish(EVENT_BUS_KIND_VDFS, None, json!({ "n": 1 }));
    EventBus::try_publish(EVENT_BUS_KIND_VDFS, None, json!({ "n": 2 })); // Full → 触发补送
    let _ = rx.recv().await; // 腾出空间

    // 全局总线：并发测试的帧也可能落进来，故「读到标记为止」而不是「下一帧就是标记」。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_marker = false;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(PluginFrame::Data(v)))
                if v["data"]["data"]["type"] == EVENT_BUS_RESYNC_MARKER_TYPE =>
            {
                saw_marker = true;
                break;
            }
            Ok(Some(_)) => continue,
            _ => break,
        }
    }

    assert!(
        saw_marker,
        "满通道后必须补送 resync 标记，否则消费端无从自愈"
    );
    event_bus_unregister_subscriber(&id);
}

/// 从通道里挑出带指定 `tag` 的帧，返回**载荷的 `Arc`**。
///
/// 订阅表是进程级的、用例并行跑：本通道也会收到其它用例发布的事件，
/// 因此靠载荷里唯一的 `tag` 认领自己的那一帧。
fn take_bus_frame(rx: &mut mpsc::Receiver<PluginFrame>, tag: &str) -> Option<Arc<Value>> {
    while let Ok(frame) = rx.try_recv() {
        let PluginFrame::Data(v) = frame else {
            continue;
        };
        if v["data"]["data"]["tag"].as_str() == Some(tag) {
            return Some(v);
        }
    }
    None
}

/// **扇出不复制载荷**——`PluginFrame::Data(Arc<Value>)` 存在的全部理由。
///
/// VDFS 变更可能同时送给多个订阅者（Tauri 前端 + 各 CLI + 子智能体桥），
/// 每个订阅者拿到的必须是**同一份**载荷分配（引用计数自增），而不是各一份深拷贝。
#[tokio::test]
async fn fan_out_shares_one_payload_allocation() {
    let _g = SERIAL.lock().await;
    let tag = format!("fanout-{}", uuid::Uuid::new_v4());
    let (tx1, mut rx1) = mpsc::channel(512);
    let (tx2, mut rx2) = mpsc::channel(512);
    let (id1, id2) = (format!("{tag}-a"), format!("{tag}-b"));
    event_bus_register_subscriber(id1.clone(), tx1);
    event_bus_register_subscriber(id2.clone(), tx2);

    EventBus::try_publish(EVENT_BUS_KIND_VDFS, None, json!({ "tag": tag }));

    let a = take_bus_frame(&mut rx1, &tag).expect("订阅者 A 应收到事件");
    let b = take_bus_frame(&mut rx2, &tag).expect("订阅者 B 应收到事件");
    assert!(
        Arc::ptr_eq(&a, &b),
        "两个订阅者必须共享同一份载荷分配；拿到两份说明扇出路径在做深拷贝"
    );

    event_bus_unregister_subscriber(&id1);
    event_bus_unregister_subscriber(&id2);
}
