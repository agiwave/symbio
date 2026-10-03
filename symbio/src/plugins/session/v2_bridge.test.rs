//! v2 事实桥验收——转写的事件格必须与 core 侧事实源**同构**：不变量全绿、
//! 溯源（N5）、轮次编号（N3）靠构造成立，重开恢复（N2）不因桥而破。

use super::{first_user_utterance, last_assistant_text, record, record_to_wal, V2Closure};
use crate::symbio_core::chat_message as cm;
use crate::symbio_core::{
    check_all, Entity, EventEnvelope as _, EventWalStore, Seq, Store, EVENT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE,
};
use std::path::PathBuf;

fn tmp_wal(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-v2bridge-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录");
    dir.join("v2-events.wal")
}

fn user_text(id: &str, text: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        role: Some(cm::MessageRole::User),
        msg_type: Some(cm::MessageType::Text),
        status: Some(cm::MessageStatus::Completed),
        content: Some(cm::MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

fn assistant_text(id: &str, text: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        role: Some(cm::MessageRole::Assistant),
        msg_type: Some(cm::MessageType::Text),
        status: Some(cm::MessageStatus::Completed),
        content: Some(cm::MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 成功轮：两格（用户开 + final 收），溯源指向本轮用户格，实测耗时入账。
#[test]
fn final_closure_writes_user_and_final_with_cost() {
    let wal = tmp_wal("final");
    record_to_wal(
        wal.clone(),
        "u-1",
        "你好",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 4321,
        },
    )
    .expect("转写成功");

    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 2, "用户格 + final 格");
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    let user = &snap[0];
    assert_eq!(user.kind, EVENT_USER_MESSAGE);
    assert_eq!(user.entity, Entity::Turn);
    assert_eq!(user.turn, 0);
    assert_eq!(
        user.payload.get("text").and_then(|v| v.as_str()),
        Some("你好")
    );
    assert_eq!(
        user.payload.get("tier").and_then(|v| v.as_str()),
        Some("deep")
    );
    assert_eq!(
        user.payload.get("turn_ref").and_then(|v| v.as_str()),
        Some("u-1")
    );

    let fin = &snap[1];
    assert_eq!(fin.kind, EVENT_ASSISTANT_FINAL);
    assert_eq!(fin.turn, 0);
    assert_eq!(
        fin.produced_by,
        user.seq().map(|s| s.value()),
        "final 溯源指向本轮用户格（N5）"
    );
    assert_eq!(fin.cost_ms, 4321, "实测耗时随事件入账（ADR-044）");
    assert_eq!(
        fin.payload.get("model").and_then(|v| v.as_str()),
        Some("v1-chat_loop")
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 轮次与重试编号：新消息推 turn；同一消息（resume 重试）推 attempt 且 id 不撞。
#[test]
fn retry_of_same_message_increments_attempt_and_new_message_increments_turn() {
    let wal = tmp_wal("retry");
    record_to_wal(
        wal.clone(),
        "u-1",
        "问",
        V2Closure::Fallback {
            why: "上游 500".into(),
            cost_ms: 100,
        },
    )
    .unwrap();
    record_to_wal(
        wal.clone(),
        "u-1",
        "问",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 200,
        },
    )
    .expect("重试转写成功（id 不撞 = 幂等键不冲突）");
    record_to_wal(
        wal.clone(),
        "u-2",
        "问2",
        V2Closure::Final {
            text: "答2".into(),
            cost_ms: 300,
        },
    )
    .unwrap();

    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 6, "三轮 × 两格");
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    // turn 编号：0 / 1 / 2（每个 user.message 都是新轮次）。
    let turns: Vec<u64> = snap
        .iter()
        .filter(|e| e.kind == EVENT_USER_MESSAGE)
        .map(|e| e.turn)
        .collect();
    assert_eq!(turns, vec![0, 1, 2]);

    // attempt：同一 v1 消息 id（u-1）两次 → 第二次 attempt=1；u-2 首次 attempt=0。
    let ids: Vec<&str> = snap.iter().map(|e| e.event_id()).collect();
    assert!(ids.contains(&"v2u-u-1-a0") && ids.contains(&"v2fb-u-1-a0"));
    assert!(ids.contains(&"v2u-u-1-a1") && ids.contains(&"v2f-u-1-a1"));
    assert!(ids.contains(&"v2u-u-2-a0") && ids.contains(&"v2f-u-2-a0"));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 重开恢复（N2）：关掉再开，事件还在、seq 完好、不变量照绿。
#[test]
fn reopen_recovers_events_with_seq() {
    let wal = tmp_wal("reopen");
    record_to_wal(
        wal.clone(),
        "u-1",
        "问",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 5,
        },
    )
    .unwrap();
    let count_before = EventWalStore::open(&wal).unwrap().range(Seq::new(0)).len();
    drop(EventWalStore::open(&wal).unwrap());

    let reopened = EventWalStore::open(&wal).unwrap();
    let snap = reopened.range(Seq::new(0));
    assert_eq!(snap.len(), count_before, "重开后事件不丢");
    assert!(
        snap.iter().all(|e| e.seq().is_some()),
        "重开恢复的事件必须带 seq"
    );
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 发言抽取：只认已完成的用户 Text 节点（流式中/工具节点不冒充发言）。
#[test]
fn utterance_helpers_pick_completed_text_nodes_only() {
    let mut streaming = user_text("s", "流式中");
    streaming.status = Some(cm::MessageStatus::Streaming);
    let tool = cm::ChatMessage {
        id: "t".into(),
        role: Some(cm::MessageRole::Assistant),
        msg_type: Some(cm::MessageType::ToolCall),
        status: Some(cm::MessageStatus::Completed),
        ..Default::default()
    };

    let msgs = vec![
        streaming,
        tool,
        user_text("u1", "第一问"),
        assistant_text("a1", "第一答"),
        user_text("u2", "补充"),
        assistant_text("a2", "第二答"),
    ];

    let (id, text) = first_user_utterance(&msgs).expect("有用户发言");
    assert_eq!(
        (id.as_str(), text.as_str()),
        ("u1", "第一问"),
        "取本轮第一条"
    );
    assert_eq!(
        last_assistant_text(&msgs).as_deref(),
        Some("第二答"),
        "取最后一条"
    );
}

/// 无发言 / 无答复的空轮形态：None → Fallback（诚实缺口）。
#[test]
fn empty_messages_yield_no_utterance() {
    let msgs: Vec<cm::ChatMessage> = vec![];
    assert!(first_user_utterance(&msgs).is_none());
    assert!(last_assistant_text(&msgs).is_none());
}

/// 总开关：`off` 档不转写——不建 WAL、网格零增长（用户关的是数据源，不是对话）；
/// `bridge` 档照常落格。锁与目录都从真会话走（`record` 的唯一判据是 `v2_mode`）。
#[tokio::test]
async fn v2_mode_off_disables_recording() {
    use std::sync::Arc;
    use tokio::sync::RwLock;

    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(super::super::store::SessionStore::new(
        tmp.path().to_path_buf(),
    ));
    let session_id = "s-switch".to_string();
    // 每个用到的会话 id 都要先落盘种子（session_dir 只认已存在的会话目录）。
    for id in [&session_id, "s-off", "s-on"] {
        store
            .save_session(&super::super::types::Session::new(id))
            .await
            .expect("种子会话落盘");
    }

    // off 档：record 后不建 WAL。
    let cfg = super::super::config::SessionConfig {
        v2_mode: super::super::config::V2Mode::Off,
        ..Default::default()
    };
    let off_session = super::super::chat_session::PersistentChatSession::new(
        "s-off".to_string(),
        Arc::new(RwLock::new(cfg)),
        store.clone(),
    );
    record(
        &off_session,
        "u-1",
        "问",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 1,
        },
    );
    let dir = off_session.session_dir().expect("持久会话有目录");
    assert!(!dir.join("v2-events.wal").exists(), "off 档不得写 WAL");

    // bridge 档（出厂默认）：照常落格。
    let on_session = super::super::chat_session::PersistentChatSession::new(
        "s-on".to_string(),
        Arc::new(RwLock::new(super::super::config::SessionConfig::default())),
        store,
    );
    record(
        &on_session,
        "u-1",
        "问",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 1,
        },
    );
    let dir = on_session.session_dir().expect("持久会话有目录");
    assert!(dir.join("v2-events.wal").exists(), "bridge 档必须写 WAL");
}
