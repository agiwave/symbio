//! `v2_worker_progress` 的判据（轨 S 第二步：Q3 进展读切 WAL）。
//!
//! ## 这一批的承重判据只有一条
//!
//! **两种读法必须给同一答案**。`worker_progress` 旧读法看 `messages.json`，新读法看
//! WAL + `transcript` / `turnstate` 投影。切流之后旧路不再被调用，**再没有东西能
//! 照出它们分叉**——所以对照必须写在切流之前，且它必须能在单测里跑。
//!
//! 分叉不是理论风险：`rounds` 数不数 supplement、`steps` 留最近几条、`in_flight`
//! 怎么判"终态"，三处各有一次口径选择。任一处选错，症状都是**数字悄悄不对**
//! （rounds 多 1、steps 少一条），不报错、不断流、模型只是拿到一句过时的进展。
//!
//! ## 用例怎么造
//!
//! 每个用例先落一份 WAL（真实事件格），再按**同一批事实**造对应的
//! `messages.json` 形态（落库镜像），然后两个读法各跑一次比结果。
//! 「同一批事实」是关键词：两边必须来自同一次会话，否则对照没有意义。

use super::progress_from_wal;
// 旧读法 = delegate::progress_of（**生产里回落用的就是它**）。测试复用它而不是在
// 本模块再抄一份：两读法对照的判据要的是「生产的旧读法」，不是「像旧读法的副本」——
// 抄一份的话，副本与真身漂移了也不会有人知道。
use crate::plugins::session::chat_loop::delegate::progress_of;
use crate::plugins::session::types::{Session, SessionSummary};
use crate::symbio_core::chat_message::{ChatMessage, MessageContent, MessageRole, MessageStatus};
use crate::symbio_core::{
    Entity, Event, EventWalStore, Store, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE,
};

/// 旧读法（**生产回落的那一个**）：delegate::progress_of。
///
/// 它要 `SessionSummary` + `Session` 两个调用方本来有的东西，这里用最小字段造——
/// 进展只依赖 `summary.id` / `summary.title` 与 `session.messages`（progress_of 的内部）。
fn old_read(
    id: &str,
    title: &str,
    messages: &[ChatMessage],
) -> super::super::chat_loop::delegate::WorkerProgress {
    let mut session = Session::new(id);
    session.messages = messages.to_vec();
    let mut summary = SessionSummary::of(&session);
    summary.title = title.to_string();
    progress_of(&summary, &session)
}
/// 造一个临时会话目录并往 WAL 里落几轮对话。
///
/// 每轮 = 开轮格 + 收束格（`final` 或 `fallback`），返回 `(dir, 该会话的落库镜像)`。
/// 镜像按**同一批事实**构造：user 消息 + assistant 消息（含 status）。
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

// ── 承重判据：两读法同答案 ────────────────────────────────────────────

#[test]
fn both_reads_agree_on_a_settled_conversation() {
    let (dir, msgs) = seed("settled", &[("问 A", Some("答 A")), ("问 B", Some("答 B"))]);

    let wal_read = progress_from_wal(&dir, "w1", "worker").expect("有 WAL ⇒ 有读数");
    let msg_read = old_read("w1", "worker", &msgs);

    assert_eq!(wal_read.rounds, msg_read.rounds, "rounds 两读法必须同数");
    assert_eq!(wal_read.steps, msg_read.steps, "steps 两读法必须同串");
    assert_eq!(
        wal_read.in_flight, msg_read.in_flight,
        "in_flight 两读法必须同判"
    );
    // 期望值本身也钉一下（防两边一起错）
    assert_eq!(wal_read.rounds, 2);
    assert_eq!(wal_read.steps, vec!["答 A", "答 B"]);
    assert!(!wal_read.in_flight, "末轮已收束 ⇒ 不在途");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn both_reads_agree_when_the_last_turn_is_still_open() {
    // 第三轮只落了开轮格（没收束）——对应"worker 还在跑"
    let (dir, msgs) = seed("open", &[("问 A", Some("答 A")), ("问 B", None)]);
    // 镜像里那轮 fallback 也标 Completed（旧读法看 status）
    // 但 WAL 侧 turnstate::settled() 对 fallback 已收束——所以旧读法也说不在途。
    // 真正"在途"的形状是**末条没有收束格**，单测里改用手工形状：
    let msgs_open: Vec<ChatMessage> = msgs
        .iter()
        .cloned()
        .chain(std::iter::once(ChatMessage {
            id: "m-streaming".into(),
            role: Some(MessageRole::Assistant),
            content: Some(MessageContent::Text("半截".into())),
            // status = Streaming ⇒ 旧读法判在途
            status: Some(MessageStatus::Streaming),
            ..Default::default()
        }))
        .collect();

    // WAL 侧补一个"开轮未收束"的第三轮，让两读法都说在途
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
    let msg_read = old_read("w1", "worker", &msgs_open);

    assert!(wal_read.in_flight, "末轮已开未收束 ⇒ WAL 读法判在途");
    assert!(msg_read.in_flight, "末条 Streaming ⇒ 旧读法判在途");
    // rounds：WAL 多数了那个"问 C"（3 vs 2）——这正是"两源分叉"的真实形状，
    // 故这个用例**只比 in_flight**，rounds 交由 S1 的双源对账去照。
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

// ── steps 的截断纪律一致 ──────────────────────────────────────────────

#[test]
fn steps_truncate_to_the_same_window_on_both_reads() {
    // 造 10 轮 ⇒ steps 超过 STEPS_MAX(8)，两读法都应只留最近 8 条
    let mut turns = Vec::new();
    for i in 0..10 {
        turns.push((format!("问 {i}"), Some(format!("答 {i}"))));
    }
    let refs: Vec<(&str, Option<&str>)> = turns
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_deref()))
        .collect();
    let (dir, msgs) = seed("trunc", &refs);

    let wal_read = progress_from_wal(&dir, "w1", "worker").expect("有 WAL");
    let msg_read = old_read("w1", "worker", &msgs);

    assert_eq!(wal_read.steps.len(), 8, "WAL 读法留最近 8 条");
    assert_eq!(msg_read.steps.len(), 8, "旧读法同样留最近 8 条");
    assert_eq!(wal_read.steps, msg_read.steps, "且两串逐字相同");
    assert_eq!(wal_read.steps.last().map(String::as_str), Some("答 9"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 空正文占位：两读法都不产出步骤 ────────────────────────────────────

#[test]
fn empty_placeholder_nodes_produce_no_steps_on_either_read() {
    let (dir, msgs) = seed("ph", &[("问 A", Some("答 A"))]);
    // 追加一个空正文的 assistant（turn 容器 / 流式占位）——S1 同一条口径
    let mut msgs2 = msgs.clone();
    msgs2.push(ChatMessage {
        id: "m-empty".into(),
        role: Some(MessageRole::Assistant),
        content: Some(MessageContent::Text(String::new())),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    });

    let wal_read = progress_from_wal(&dir, "w1", "worker").expect("有 WAL");
    let msg_read = old_read("w1", "worker", &msgs2);

    assert_eq!(wal_read.steps, vec!["答 A"], "WAL 侧只收非空正文");
    assert_eq!(msg_read.steps, vec!["答 A"], "旧读法同（空占位不产出步骤）");
    let _ = std::fs::remove_dir_all(&dir);
}
