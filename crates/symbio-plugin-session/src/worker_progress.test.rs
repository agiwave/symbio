//! `worker_progress` 的判据（Q3 进展读切 WAL）。
//!
//! ## 之后这里钉什么
//!
//! 本文件原先的承重判据是「**两种读法必须给同一答案**」——旧读法看落库消息、新读法看
//! WAL，两个源并排跑。 退掉了那份对账：切源之后落库消息**就是** WAL 重建的那
//! 一份（`store::load_messages` 已 WAL 优先），两个源不再独立，对照失去对象。
//!
//! 故本文件改为**直接钉事实读法的取值**：`rounds` / `steps` / `in_flight` 三个数
//! 各自对得上哪一格事件（见模块头的对应表）。旧读法（[`progress_of`]）的投影判据
//! 用例自 `delegate.test.rs` 随实现同住本文件（[D-081](../../../docs/003-design/014-单环一表.md#d-081-p4驱动器不解存储不把存储内部对象拿回家构造只住在收敛手柄批次-6-分-6a--6b "DE-014-单环一表")
//! 裁决一第 4 条：进展的类型与读法归本模块）。

use super::{progress_from_facts, progress_of, STEPS_MAX, STEP_MAX_CHARS};

/// 身份写 `principal`（D-147）：夹具模仿的是**落库面**的消息，`role` 只是读侧残留格。
fn principal_for(role: cm::NodeRole) -> symbio_core::principal::Principal {
    use symbio_core::principal::Principal;
    match role {
        cm::NodeRole::User => Principal::from_legacy(symbio_core::PRINCIPAL_USER),
        cm::NodeRole::Assistant => Principal::from_legacy("session/main"),
        cm::NodeRole::Tool => Principal::from_legacy("legacy/unknown"),
        cm::NodeRole::System => Principal::from_legacy("system/local"),
    }
}
use crate::store::{EventWalStore, SessionStore};
use crate::types::{Session, SessionSummary};
use symbio_core::chat_node as cm;
use symbio_core::chat_node::{ChatNode, MessageContent, NodeStatus};
use symbio_core::{Entity, Event, Seq, Store, Verb};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};

/// 会话事实的**值**（生产侧由 `SessionStore::snapshot` 交回）。
fn facts_of(dir: &std::path::Path) -> Option<Vec<Event>> {
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    if !wal.is_file() {
        return None;
    }
    let store = EventWalStore::open_readonly(&wal).ok()?;
    Some(store.range(Seq::new(0)))
}

/// 造一个临时会话目录并往 WAL 里落几轮对话。
///
/// 每轮 = 开轮格 + 收束格（`final` 或 `fallback`）。返回 `(dir, 该会话的落库镜像)`；
/// 镜像按**同一批事实**构造（user + assistant 含 status），供落库侧口径的用例使用。
fn seed(tag: &str, turns: &[(&str, Option<&str>)]) -> (std::path::PathBuf, Vec<ChatNode>) {
    let dir = std::env::temp_dir().join(format!("symbio-wprog-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).expect("open wal");

    let mut msgs: Vec<ChatNode> = Vec::new();
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
                            EVENT_CHAT_ASSISTANT_FINAL,
                            Entity::Turn,
                            Verb::Closed,
                            t,
                            "session/main",
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
                            EVENT_CHAT_ASSISTANT_FALLBACK,
                            Entity::Turn,
                            Verb::Closed,
                            t,
                            "session/main",
                        )
                        .with_produced_by(s)
                        .with_payload(serde_json::json!({ "why": "impl 失败" })),
                    )
                    .expect("fallback");
            }
        }
        // 落库镜像：user + assistant（Completed）——与上面两格同一批事实
        msgs.push(ChatNode {
            id: format!("m-u-{tag}-{t}"),
            principal: Some(principal_for(cm::NodeRole::User)),
            content: Some(MessageContent::Text((*u).to_string())),
            status: Some(NodeStatus::Completed),
            ..Default::default()
        });
        msgs.push(ChatNode {
            id: format!("m-a-{tag}-{t}"),
            principal: Some(principal_for(cm::NodeRole::Assistant)),
            content: Some(MessageContent::Text(a.unwrap_or("impl 失败").to_string())),
            status: Some(NodeStatus::Completed),
            ..Default::default()
        });
    }
    (dir, msgs)
}

// ── 事实读法的取值 ────────────────────────────────────────────────────

#[test]
fn a_settled_conversation_reads_rounds_steps_and_not_in_flight() {
    let (dir, _msgs) = seed("settled", &[("问 A", Some("答 A")), ("问 B", Some("答 B"))]);

    let wal_read = progress_from_facts(&facts_of(&dir).expect("有事实源"), "w1", "worker");

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

    let wal_read = progress_from_facts(&facts_of(&dir).expect("有事实源"), "w1", "worker");

    assert!(wal_read.in_flight, "末轮已开未收束 ⇒ 判在途");
    assert_eq!(wal_read.rounds, 3, "第三轮的开轮格也计入 rounds");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 无事实源 ⇒ 存储层交 None，调用方回落旧读法 ──────────────────────

#[test]
fn no_wal_yields_no_reading_and_the_caller_falls_back() {
    let root = std::env::temp_dir().join(format!("symbio-wprog-nowal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("nowal");
    std::fs::create_dir_all(&dir).expect("临时目录");

    // 生产入口 = 存储层：没有事实源 ⇒ `None`（调用方回落旧读法，不造空壳）。
    let store = SessionStore::new(root);
    assert!(
        store.snapshot("nowal").is_none(),
        "没有 WAL ⇒ None（不打开、不创建任何文件）"
    );
    assert!(
        !dir.join(super::super::paths::V2_WAL_FILE).exists(),
        "读方不创建文件"
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

    let wal_read = progress_from_facts(&facts_of(&dir).expect("有事实源"), "w1", "worker");

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
                EVENT_CHAT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                // 另一个轮：同一轮的第二条 final 会被写侧闸门拒掉（I1），而本测试要
                // 验的是「空正文不产出步骤」，不是「重复收束」。
                1,
                "session/main",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "text": "" })),
        )
        .expect("empty placeholder");
    drop(store);

    let wal_read = progress_from_facts(&facts_of(&dir).expect("有事实源"), "w1", "worker");

    assert_eq!(wal_read.steps, vec!["答 A"], "空占位不产出步骤");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Q3 · worker 进展（三个字段与 B2 逐字同名） ─────────────────────────

/// 夹具：一条 worker 消息。
fn wmsg(id: &str, role: cm::NodeRole, status: Option<cm::NodeStatus>, text: &str) -> cm::ChatNode {
    cm::ChatNode {
        id: id.to_string(),
        principal: Some(principal_for(role)),
        msg_type: Some(cm::NodeKind::Body),
        status,
        content: Some(cm::MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 夹具：子会话清单条目。`message_count` **故意与真实 Turn 数对不上**——它是总消息数，
/// 投影若拿它顶 `rounds`，本文件的断言立刻红。
fn sub(id: &str, title: &str, message_count: usize) -> SessionSummary {
    SessionSummary {
        id: id.to_string(),
        title: title.to_string(),
        created_at: 0,
        updated_at: 0,
        metadata: serde_json::json!({ "parent_session_id": "parent-1" }),
        message_count,
        summary: None,
        meta_tags: Vec::new(),
        context: None,
    }
}

/// 夹具：worker 的完整转写。
fn wsession(messages: Vec<cm::ChatNode>) -> Session {
    Session {
        id: "worker-1".to_string(),
        messages,
        created_at: 0,
        updated_at: 0,
        metadata: serde_json::json!({ "parent_session_id": "parent-1" }),
    }
}

#[test]
fn rounds_counts_user_messages_not_the_summary_count() {
    // 判据 2「数值同源」的落点：前端数过程段 Turn 条数，后端数 user 消息——同一个量。
    let s = wsession(vec![
        wmsg(
            "1",
            cm::NodeRole::User,
            Some(cm::NodeStatus::Completed),
            "第一轮",
        ),
        wmsg(
            "2",
            cm::NodeRole::Assistant,
            Some(cm::NodeStatus::Completed),
            "答复一",
        ),
        wmsg(
            "3",
            cm::NodeRole::User,
            Some(cm::NodeStatus::Completed),
            "第二轮",
        ),
        wmsg(
            "4",
            cm::NodeRole::Assistant,
            Some(cm::NodeStatus::Completed),
            "答复二",
        ),
    ]);
    let p = progress_of(&sub("worker-1", "重构", 99), &s);
    assert_eq!(p.rounds, 2, "rounds 必须是 user 消息数");
    assert_ne!(
        p.rounds, 99,
        "拿 summary.message_count 当 rounds ⇒ 与 B2 不同源（不同量）"
    );
}

#[test]
fn steps_skip_empty_nodes_and_user_prompts() {
    // 判据 3：无内容节点不产出步骤；user 提示是**输入**不是进展。
    let grouping = cm::ChatNode {
        id: "2".to_string(),
        msg_type: Some(cm::NodeKind::Skeleton),
        meta: Some(serde_json::json!({"style_tag": "run"})),
        ..Default::default()
    };
    let empty = cm::ChatNode {
        id: "3".to_string(),
        principal: Some(principal_for(cm::NodeRole::Assistant)),
        msg_type: Some(cm::NodeKind::Body),
        content: Some(cm::MessageContent::Text(String::new())),
        ..Default::default()
    };
    let s = wsession(vec![
        wmsg(
            "1",
            cm::NodeRole::User,
            Some(cm::NodeStatus::Completed),
            "干活",
        ),
        grouping,
        empty,
        wmsg(
            "4",
            cm::NodeRole::Assistant,
            Some(cm::NodeStatus::Completed),
            "产出",
        ),
    ]);
    let p = progress_of(&sub("worker-1", "重构", 0), &s);
    assert_eq!(
        p.steps,
        vec!["产出".to_string()],
        "空节点或用户提示都不得产出步骤"
    );
}

#[test]
fn in_flight_is_false_only_once_a_terminal_state_is_seen() {
    let done = wsession(vec![wmsg(
        "1",
        cm::NodeRole::Assistant,
        Some(cm::NodeStatus::Completed),
        "完",
    )]);
    assert!(!progress_of(&sub("worker-1", "t", 0), &done).in_flight);

    let running = wsession(vec![wmsg(
        "1",
        cm::NodeRole::Assistant,
        Some(cm::NodeStatus::Streaming),
        "写",
    )]);
    assert!(progress_of(&sub("worker-1", "t", 0), &running).in_flight);

    // `status` 缺失 = 没观测到结束 ⇒ 不得宣称已结束。谎报"已完成"会让模型抢跑，
    // 比多报在途糟得多（同「串台比没进展更糟」的取向）。
    let no_status = wsession(vec![cm::ChatNode {
        id: "1".to_string(),
        ..Default::default()
    }]);
    assert!(
        progress_of(&sub("worker-1", "t", 0), &no_status).in_flight,
        "status 缺失被判成终态 ⇒ 所有存量会话都显示「已结束」"
    );
}

#[test]
fn steps_keep_the_recent_capped_and_truncated() {
    // 进展关心"到哪了"，不是"都干过什么"：超上限丢最早的、单步超长截断——
    // 否则每轮请求都要背着 worker 的全部历史。
    let long = "段".repeat(STEP_MAX_CHARS + 40);
    let messages: Vec<cm::ChatNode> = (0..STEPS_MAX + 3)
        .map(|i| {
            wmsg(
                &format!("m{i}"),
                cm::NodeRole::Assistant,
                Some(cm::NodeStatus::Completed),
                &format!("{i}:{long}"),
            )
        })
        .collect();
    let p = progress_of(&sub("worker-1", "t", 0), &wsession(messages));
    assert_eq!(p.steps.len(), STEPS_MAX, "超上限必须丢最早的");
    assert!(
        p.steps[0].starts_with("3:"),
        "丢错了端——应保留最近的（首条应为第 3 条），实际：{}",
        p.steps[0]
    );
    assert!(
        p.steps[0].chars().count() <= STEP_MAX_CHARS + 1,
        "单步未截断（含省略号至多 {} 字）",
        STEP_MAX_CHARS + 1
    );
}
