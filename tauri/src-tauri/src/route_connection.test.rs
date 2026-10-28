//! `route_connection.rs` 的单元测试 —— 连接生命周期的最小判据。
//!
//! 这里守的是「前端消失之后后端不留下孤儿」：超时清理只该摘掉闲着的连接，
//! 且摘除的同时必须取消订阅令牌（否则订阅表要等下一次发布才收口）。

use super::*;
use serde_json::json;

fn channel() -> (mpsc::Sender<PluginFrame>, mpsc::Receiver<PluginFrame>) {
    mpsc::channel(4)
}

#[tokio::test]
async fn sweep_expired_removes_only_idle_connections_and_cancels_their_tokens() {
    let mgr = RouteConnectionManager::new();
    let (tx_old, _rx_old) = channel();
    let (tx_fresh, _rx_fresh) = channel();
    let old = CancellationToken::new();
    let fresh = CancellationToken::new();
    mgr.register_fixed("old".into(), tx_old, old.clone()).await;
    mgr.register_fixed("fresh".into(), tx_fresh, fresh.clone())
        .await;

    // 时间轴整个放在基准**之后**：`Instant` 的零点 = 开机，往回拨 301s 在开机不满
    // 301s 的机器上是 panic 而不是夹具（见 `sweep_expired` 的 `now`）。被审的仍是差值：
    // 判超时的那一刻距 `old` 的活动 301s（超窗口）、距 `fresh` 的 51s（在窗口内）。
    let base = Instant::now();
    {
        let mut conns = mgr.connections.write().await;
        conns.get_mut("old").expect("刚注册的连接").last_active = base;
        conns.get_mut("fresh").expect("刚注册的连接").last_active = base + Duration::from_secs(250);
    }

    sweep_expired(
        &mgr.connections,
        Duration::from_secs(300),
        base + Duration::from_secs(301),
    )
    .await;

    assert!(old.is_cancelled(), "超时连接必须取消订阅令牌");
    assert!(!fresh.is_cancelled(), "未超时的连接不得被误删");
    assert!(
        mgr.send("old", PluginFrame::data(json!({}))).await.is_err(),
        "超时连接已摘除，send 必须失败而不是写进一个没人收的通道"
    );
    assert!(mgr
        .send("fresh", PluginFrame::data(json!({})))
        .await
        .is_ok());
}

#[tokio::test]
async fn remove_all_drops_every_connection_and_cancels_their_tokens() {
    let mgr = RouteConnectionManager::new();
    let (tx_a, _rx_a) = channel();
    let (tx_b, _rx_b) = channel();
    let a = CancellationToken::new();
    let b = CancellationToken::new();
    mgr.register_fixed("a".into(), tx_a, a.clone()).await;
    mgr.register_fixed("b".into(), tx_b, b.clone()).await;

    mgr.remove_all().await;

    assert!(
        a.is_cancelled() && b.is_cancelled(),
        "移除全部必须逐个取消令牌"
    );
    assert!(mgr.connections.read().await.is_empty(), "表必须被清空");
    assert!(mgr.send("a", PluginFrame::data(json!({}))).await.is_err());
}
