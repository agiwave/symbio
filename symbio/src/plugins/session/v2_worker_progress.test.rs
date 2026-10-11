//! `v2_worker_progress` 的判据（轨 S 第二步：Q3 进展读切 WAL）。
//!
//! ## S2.7-a 之后这里钉什么
//!
//! 本文件原先的承重判据是「**两种读法必须给同一答案**」——旧读法看落库消息、新读法看
//! WAL，两个源并排跑。S2.7-a 退掉了那份对账：切源之后落库消息**就是** WAL 重建的那
//! 一份（`store::load_messages` 已 WAL 优先），两个源不再独立，对照失去对象。
//!
//! 故本文件改为**直接钉 WAL 读法的取值**：`rounds` / `steps` / `in_flight` 三个数
//! 各自对得上哪一格事件（见模块头的对应表）。旧读法（`delegate::progress_of`）仍在
//! 「没有 WAL 的会话」这条回落路上，但它已是另一条路径，不再拿来对照。

use super::progress_from_wal;
use crate::symbio_core::chat_message::{ChatMessage, MessageContent, MessageRole, MessageStatus};
use crate::symbio_core::{
    Entity, Event, EventWalStore, Store, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE,
};

/// 造一个临时会话目录并往 WAL 里落几轮对话。
///
/// 每轮 = 开轮格 + 收束格（`final` 或 `fallback`）。返回 `(dir, 该会话的落库镜像)`；
/// 镜像按**同一批事实**构造（user + assistant 含 status），供落库侧口径的用例使用。
fn seed(tag: &str, turns: &[(&str, Option<&str>)]) -> (std::path::PathBuf, Vec<ChatMessage>) {
    let dir = std::env::temp_dir().join(format!("symbio-wprog-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).expect("open wal");

    let mut msgs: Vec<ChatMessage> = Vec::new();
    for (i, (u, a)) in turns.iter().enumerate() {
        let t = i as u64;
        let s = store
            .append(
                Event::pending(
                    format!("u-{tag}-{t}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    t,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": u })),
            )
            .expect("user")
            .value();
        match a {
            Some(text) => {
                store
                    .append(
                        Event::pending(
                            format!("f-{tag}-{t}"),
                            EVENT_ASSISTANT_FINAL,
                            Entity::Turn,
                            Verb::Closed,
                            t,
                            "agent:main",
                        )
                        .with_produced_by(s)
                        .with_payload(serde_json::json!({ "text": text })),
                    )
                    .expect("final");
            }
            None => {
                store
                    .append(
                        Event::pending(
                            format!("fb-{tag}-{t}"),
                            EVENT_ASSISTANT_FALLBACK,
                            Entity::Turn,
                            Verb::Closed,
                            t,
                            "agent:main",
                        )
                        .with_produced_by(s)
                        .with_payload(serde_json::json!({ "why": "impl 失败" })),
                    )
                    .expect("fallback");
            }
        }
        // 落库镜像：user + assistant（Completed）——与上面两格同一批事实
        msgs.push(ChatMessage {
            id: format!("m-u-{tag}-{t}"),
            role: Some(MessageRole::User),
            content: Some(MessageContent::Text((*u).to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        });
        msgs.push(ChatMessage {
            id: format!("m-a-{tag}-{t}"),
            role: Some(MessageRole::Assistant),
            content: Some(MessageContent::Text(a.unwrap_or("impl 失败").to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        });
    }
    (dir, msgs)
}

// ── WAL 读法的取值 ────────────────────────────────────────────────────

#[test]
fn a_settled_conversation_reads_rounds_steps_and_not_in_flight() {
    let (dir, _msgs) = seed("settled", &[("问 A", Some("答 A")), ("问 B", Some("答 B"))]);

    let wal_read = progress_from_wal(&dir, "w1", "worker").expect("有 WAL ⇒ 有读数");

    assert_eq!(wal_read.rounds, 2, "rounds = user.message 条数");
    assert_eq!(
        wal_read.steps,
        vec!["答 A", "答 B"],
        "steps = 非 user 的非空正文"
    );
    assert!(!wal_read.in_flight, "末轮已收束 ⇒ 不在途");
    assert_eq!(wal_read.id, "w1");
    assert_eq!(wal_read.title, "worker");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_open_last_turn_reads_as_in_flight() {
    // 第三轮只落了开轮格（没收束）——对应"worker 还在跑"
    let (dir, _msgs) = seed("open", &[("问 A", Some("答 A")), ("问 B", Some("答 B"))]);

    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).expect("open wal");
    store
        .append(
            Event::pending(
                "u-open-2",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                2,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "问 C" })),
        )
        .expect("third open");
    drop(store);

    let wal_read = progress_from_wal(&dir, "w1", "worker").expect("有 WAL");

    assert!(wal_read.in_flight, "末轮已开未收束 ⇒ 判在途");
    assert_eq!(wal_read.rounds, 3, "第三轮的开轮格也计入 rounds");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 无 WAL ⇒ 不产出 ───────────────────────────────────────────────────

#[test]
fn no_wal_yields_no_reading_and_the_caller_falls_back() {
    let dir = std::env::temp_dir().join(format!("symbio-wprog-nowal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");

    assert!(
        progress_from_wal(&dir, "w1", "worker").is_none(),
        "没有 WAL ⇒ None（调用方回落旧读法，不造空壳）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── steps 的截断纪律 ──────────────────────────────────────────────────

#[test]
fn steps_truncate_to_the_recent_window() {
    // 造 10 轮 ⇒ steps 超过 STEPS_MAX(8)，只留最近 8 条
    let mut turns = Vec::new();
    for i in 0..10 {
        turns.push((format!("问 {i}"), Some(format!("答 {i}"))));
    }
    let refs: Vec<(&str, Option<&str>)> = turns
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_deref()))
        .collect();
    let (dir, _msgs) = seed("trunc", &refs);

    let wal_read = progress_from_wal(&dir, "w1", "worker").expect("有 WAL");

    assert_eq!(wal_read.steps.len(), 8, "只留最近 8 条");
    assert_eq!(wal_read.steps.first().map(String::as_str), Some("答 2"));
    assert_eq!(wal_read.steps.last().map(String::as_str), Some("答 9"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 空正文占位：不产出步骤 ────────────────────────────────────────────

#[test]
fn empty_placeholder_nodes_produce_no_steps() {
    // 落一个空正文的 assistant（turn 容器 / 流式占位）——空占位不算一个步骤
    let (dir, _msgs) = seed("ph", &[("问 A", Some("答 A"))]);
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).expect("open wal");
    store
        .append(
            Event::pending(
                "p-empty",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_payload(serde_json::json!({ "text": "" })),
        )
        .expect("empty placeholder");
    drop(store);

    let wal_read = progress_from_wal(&dir, "w1", "worker").expect("有 WAL");

    assert_eq!(wal_read.steps, vec!["答 A"], "空占位不产出步骤");
    let _ = std::fs::remove_dir_all(&dir);
}
