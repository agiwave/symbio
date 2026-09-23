//! `inbox.rs` 的单元测试（入队 / 取消 / 清空 / 忙则排队）。
//!
//! ## 这里**不**测"跑一轮"
//!
//! 真跑一轮要整棵树（能力收集 + 模型 provider），那是 e2e 的活（`e2e/cases/t15-*.mjs`）。
//! 本文件锁的是队列本身的规则：顺序、身份、两种取消的分界、**忙则排队**。
//! 后者是 ADR-026 的核心承诺之一，而它恰好**能**在单测里钉死——因为它判的是
//! "这一趟要不要动队列"，与跑得成跑不成无关。

use super::*;
use crate::plugins::session::plugin::SessionConfig;
use crate::symbio_core::schemas::session::session_chat;
use crate::symbio_core::SimpleRequest;

fn plugin() -> Arc<SessionPlugin> {
    Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        crate::plugins::session::test_dir(),
    ))
}

fn user_message(text: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        role: Some(cm::MessageRole::User),
        msg_type: Some(cm::MessageType::Text),
        content: Some(cm::MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

#[tokio::test]
async fn enqueue_preserves_fifo_order_and_address_identity() {
    let p = plugin();
    p.enqueue_inbox(
        "s1",
        None,
        user_message("第一条"),
        session_chat::Request::default(),
        None,
    )
    .await;
    let second = p
        .enqueue_inbox(
            "s1",
            Some("i2".to_string()),
            user_message("第二条"),
            session_chat::Request::default(),
            None,
        )
        .await;

    // 具名条目：**地址末段即身份**——条目 id 与消息 id 是同一个值，
    // 两处各生成一个会让"按地址取消息"在两套 id 之间对不上
    assert_eq!(second.id, "i2");
    assert_eq!(second.message.id, "i2");

    let items = p.inbox_items("s1").await;
    assert_eq!(items.len(), 2);
    assert_eq!(
        items
            .iter()
            .map(|i| i.message.content.clone().unwrap().to_text())
            .collect::<Vec<_>>(),
        vec!["第一条".to_string(), "第二条".to_string()],
        "入队顺序就是消费顺序（FIFO）"
    );
    // 未具名的那条：id 由 provider 生成，且与消息 id 一致
    assert_eq!(items[0].id, items[0].message.id);
    assert!(!items[0].id.is_empty());
}

#[tokio::test]
async fn cancel_and_clear_only_affect_queued_items() {
    let p = plugin();
    for (id, text) in [("i1", "一"), ("i2", "二"), ("i3", "三")] {
        p.enqueue_inbox(
            "s1",
            Some(id.to_string()),
            user_message(text),
            session_chat::Request::default(),
            None,
        )
        .await;
    }

    assert!(p.cancel_inbox_item("s1", "i2").await, "排队中的条目可取消");
    assert!(
        !p.cancel_inbox_item("s1", "i2").await,
        "取消两次是两次动作：第二次找不到（调用方据此报 NotFound，不静默成功）"
    );
    assert!(
        !p.cancel_inbox_item("s1", "nope").await,
        "不存在的条目同样报找不到"
    );

    let ids: Vec<String> = p
        .inbox_items("s1")
        .await
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(ids, vec!["i1".to_string(), "i3".to_string()]);

    let cleared = p.clear_inbox("s1").await;
    assert_eq!(cleared, vec!["i1".to_string(), "i3".to_string()]);
    assert!(p.inbox_items("s1").await.is_empty());
}

/// 忙则排队：会话正在跑时，消费者**不能**动队列（不合并、不抢占、不并发）。
#[tokio::test]
async fn busy_session_keeps_item_queued() {
    let p = plugin();
    p.enqueue_inbox(
        "s1",
        Some("i1".to_string()),
        user_message("排队等我"),
        session_chat::Request::default(),
        None,
    )
    .await;

    // 手工把会话置为"正在跑"（真实路径由 `emit_session_state(Working)` 写）
    let state = p.active_mgr.get_or_create("s1").await;
    state.inner.write().await.is_working = true;

    let started = p.clone().drain_inbox_once().await;
    assert!(!started, "忙的时候一趟也不启动");
    assert_eq!(
        p.inbox_items("s1").await.len(),
        1,
        "条目留在队里——等这一轮收尾再取下一条"
    );

    // 空闲下来后它才被取走（本用例无父插件 ⇒ 启动失败并**放回队首**，
    // 这正是"启动失败不丢条目"的承诺；真跑一轮由 e2e 覆盖）
    state.inner.write().await.is_working = false;
    let _ = p.clone().drain_inbox_once().await;
    let items = p.inbox_items("s1").await;
    assert_eq!(items.len(), 1, "启动失败 ⇒ 原地放回，等下一趟");
    assert_eq!(items[0].id, "i1");
}

/// `run_inbox_turn` 的上下文是**自己造的**：只带目标会话与工作目录。
///
/// 发起者的 `SESSION_ID` 刻意不沿用——跨空间写入时那是发起者自己的会话
/// （父会话 → 子智能体空间），沿用会把消息投错会话。这里从**入口**侧验一次：
/// 入队带 `workdir` 时它会出现在条目上（消费者据此还原请求级上下文）。
#[tokio::test]
async fn enqueue_carries_request_params_and_workdir() {
    let p = plugin();
    let params = session_chat::Request {
        mode: Some("auto".to_string()),
        risk_level: Some("high".to_string()),
        agent_id: Some("ag1".to_string()),
        ..Default::default()
    };
    let item = p
        .enqueue_inbox(
            "s1",
            None,
            user_message("走这一份参数"),
            params,
            Some("C:/w".to_string()),
        )
        .await;
    assert_eq!(item.params.mode.as_deref(), Some("auto"));
    assert_eq!(item.params.risk_level.as_deref(), Some("high"));
    assert_eq!(item.params.agent_id.as_deref(), Some("ag1"));
    assert_eq!(item.workdir.as_deref(), Some("C:/w"));
    // 与"哪条消息发给哪个会话"无关的那三个字段不在条目参数里（各只有一个位置）
    assert!(item.params.session_id.is_none());
    assert!(item.params.message.is_none());
    assert!(item.params.resume.is_none());

    // 端到端侧的同一条规则：消费者造出的 ctx 只带目标会话 id 与 workdir
    let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    ctx.set(SESSION_ID, "s1".to_string());
    assert_eq!(ctx.get(SESSION_ID).as_deref(), Some("s1"));
    assert!(ctx.get(WORKDIR).is_none());
}
