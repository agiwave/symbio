//! `commands.rs` 的单元测试 —— 命令层转发泵的两条 EOF 路径。
//!
//! `route_v2` 本身要 `AppHandle`，但泵的业务只有三件事：帧送到、EOF 取消令牌、
//! EOF 通知前端。把「怎么发」作为闭包注入之后，这三件事可以在没有 Tauri 运行时
//! 的情况下直接钉住。

use super::spawn_forward_pump;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use symbio::symbio_core::PluginFrame;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn forward_pump_cancels_and_signals_eof_when_the_channel_closes() {
    let (tx, rx) = mpsc::channel(2);
    let cancel = CancellationToken::new();
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorder = events.clone();

    let pump = spawn_forward_pump(rx, "route/c1".into(), cancel.clone(), move |event, _| {
        recorder.lock().unwrap().push(event.to_string());
    });

    drop(tx);
    pump.await.expect("泵任务不应 panic");

    assert!(cancel.is_cancelled(), "EOF 必须取消订阅令牌");
    assert_eq!(
        &*events.lock().unwrap(),
        &["route/c1/eof".to_string()],
        "EOF 必须且只能通知一次，事件名带 /eof 后缀"
    );
}

#[tokio::test]
async fn forward_pump_delivers_concurrent_frames_before_eof() {
    let (tx, rx) = mpsc::channel(8);
    let cancel = CancellationToken::new();
    let frames = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let (count, recorder) = (frames.clone(), events.clone());

    let pump = spawn_forward_pump(
        rx,
        "route/c2".into(),
        cancel.clone(),
        move |event, frame| {
            if frame.is_some() {
                count.fetch_add(1, Ordering::SeqCst);
            }
            recorder.lock().unwrap().push(event.to_string());
        },
    );

    // 两个并发生产者各发 4 帧：泵运行期间的帧路径不得丢帧。
    let mut producers = Vec::new();
    for _ in 0..2 {
        let tx = tx.clone();
        producers.push(tokio::spawn(async move {
            for i in 0..4 {
                tx.send(PluginFrame::data(json!(i))).await.unwrap();
            }
        }));
    }
    for p in producers {
        p.await.unwrap();
    }
    drop(tx);
    pump.await.expect("泵任务不应 panic");

    assert_eq!(frames.load(Ordering::SeqCst), 8, "并发帧必须全部送达");
    let seen = events.lock().unwrap();
    assert_eq!(seen.last().map(String::as_str), Some("route/c2/eof"));
    assert!(cancel.is_cancelled());
}
