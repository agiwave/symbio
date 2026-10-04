//! v2 事实桥验收——转写的事件格必须与 core 侧事实源**同构**：不变量全绿、
//! 溯源（N5）、轮次编号（N3）靠构造成立，重开恢复（N2）不因桥而破。

use super::{
    authorize_close, first_user_utterance, last_assistant_text, record, record_to_wal, V2Closure,
};
use crate::symbio_core::chat_message as cm;
use crate::symbio_core::{
    check_all, recall, Budget, Entity, EventEnvelope as _, EventWalStore, PermissionMatrix, Seq,
    Store, VisScope, EVENT_ASSISTANT_FINAL, EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED,
    EVENT_MEMORY_FORGOTTEN, EVENT_MEMORY_RECALLED, EVENT_USER_MESSAGE,
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

/// 成功轮：两格（用户开 + final 收）+ 一条记忆（步 11 写方），溯源各自到位。
#[test]
fn final_closure_writes_user_and_final_with_cost() {
    let wal = tmp_wal("final");
    record_to_wal(
        wal.clone(),
        crate::authz::PRINCIPAL_MAIN,
        "u-1",
        "你好",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 4321,
        },
        None,
        &[],
    )
    .expect("转写成功");

    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 3, "用户格 + final 格 + 记忆格");
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

    // 记忆格（S5 步 11）：溯源指向**本轮用户格**，`ts` 是编码时刻，`vec` 恒空。
    let mem = &snap[2];
    assert_eq!(mem.kind, EVENT_MEMORY_ENCODED);
    assert_eq!(mem.entity, Entity::Memory);
    assert_eq!(
        mem.produced_by,
        user.seq().map(|s| s.value()),
        "记忆必带溯源（I2：覆盖 100%）"
    );
    assert!(mem.ts > 0, "记忆事件填真实编码时刻（RecallEntry::ts 契约）");
    assert_eq!(
        mem.payload.get("content").and_then(|v| v.as_str()),
        Some("你好")
    );
    assert!(mem
        .payload
        .get("vec")
        .is_some_and(|v| v.as_array().is_some_and(|a| a.is_empty())));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 轮次与重试编号：新消息推 turn；同一消息（resume 重试）推 attempt 且 id 不撞。
#[test]
fn retry_of_same_message_increments_attempt_and_new_message_increments_turn() {
    let wal = tmp_wal("retry");
    record_to_wal(
        wal.clone(),
        crate::authz::PRINCIPAL_MAIN,
        "u-1",
        "问",
        V2Closure::Fallback {
            why: "上游 500".into(),
            cost_ms: 100,
        },
        None,
        &[],
    )
    .unwrap();
    record_to_wal(
        wal.clone(),
        crate::authz::PRINCIPAL_MAIN,
        "u-1",
        "问",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 200,
        },
        None,
        &[],
    )
    .expect("重试转写成功（id 不撞 = 幂等键不冲突）");
    record_to_wal(
        wal.clone(),
        crate::authz::PRINCIPAL_MAIN,
        "u-2",
        "问2",
        V2Closure::Final {
            text: "答2".into(),
            cost_ms: 300,
        },
        None,
        &[],
    )
    .unwrap();

    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    // 三轮 × 两格 = 6，再加 2 条记忆：u-1 说了两遍「问」（第二次**同文去重**不记），
    // u-2 的「问2」另记一条。
    assert_eq!(snap.len(), 8, "三轮 × 两格 + 两条不重复的记忆");
    assert_eq!(
        snap.iter()
            .filter(|e| e.kind == EVENT_MEMORY_ENCODED)
            .count(),
        2,
        "同文去重：重试轮不重复编码"
    );
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
        crate::authz::PRINCIPAL_MAIN,
        "u-1",
        "问",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 5,
        },
        None,
        &[],
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

/// 真实形状回归：`chat/send` 送进来的用户消息**不填 `status`**（CLI / 前端都是
/// `..Default::default()`），它照样是本轮发言，必须被认出来。
///
/// 这条锁的故障形态很具体：判据一旦只认 `status == Completed`，转写就在真实流量
/// 上**整片消失**——没有报错、没有 `plugin_warn!`，事实源只是从来不存在，于是
/// 读数口四列全零，而全零看起来完全正常。单测里那些手造消息都带 `status`
/// （夹具替真实数据把话说满了），只有这条能替 CLI 那一支作证。
#[test]
fn user_message_without_status_is_still_this_rounds_utterance() {
    let sent = cm::ChatMessage {
        id: "u1".into(),
        role: Some(cm::MessageRole::User),
        msg_type: Some(cm::MessageType::Text),
        content: Some(cm::MessageContent::Text("普通提问".into())),
        ..Default::default()
    };
    let msgs = vec![sent, assistant_text("a1", "答")];

    let (id, text) = first_user_utterance(&msgs).expect("不填 status 的用户消息必须被认出");
    assert_eq!((id.as_str(), text.as_str()), ("u1", "普通提问"));
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
        crate::authz::PRINCIPAL_MAIN,
        "u-1",
        "问",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 1,
        },
        None,
        &[],
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
        crate::authz::PRINCIPAL_MAIN,
        "u-1",
        "问",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 1,
        },
        None,
        &[],
    );
    let dir = on_session.session_dir().expect("持久会话有目录");
    assert!(dir.join("v2-events.wal").exists(), "bridge 档必须写 WAL");
}

/// 写侧闸（[04 §3.1 批⑥](../../../../docs/plan/04-工程落地.md)）：收束入格前判
/// **能力**——首条 / 追加各判各的，未持有 ⇒ 拒绝入格（不是记一笔照写）。
#[test]
fn a_closure_without_the_grant_is_refused() {
    // 本机授权表：主智能体两种位置都放行（否则生产对话第一步就断）。
    let live = crate::authz::production_matrix();
    authorize_close(live, crate::authz::PRINCIPAL_MAIN, true).expect("首响放行");
    authorize_close(live, crate::authz::PRINCIPAL_MAIN, false).expect("追加放行");
    assert!(
        authorize_close(live, "agent:ghost", true).is_err(),
        "矩阵外主体 fail-closed"
    );

    // 只配追加能力的表 ⇒ 首条被拒、追加放行：判的是能力，不是恒真。
    let append_only = PermissionMatrix::from_names(&[(
        crate::authz::PRINCIPAL_MAIN,
        &["reply.append"],
        VisScope::ThreadPrivate,
    )])
    .expect("表合法");
    let why = authorize_close(&append_only, crate::authz::PRINCIPAL_MAIN, true)
        .expect_err("缺 reply.first ⇒ 不入格");
    assert!(why.contains("reply.first"), "{why}");
    authorize_close(&append_only, crate::authz::PRINCIPAL_MAIN, false)
        .expect("持有 reply.append ⇒ 放行");

    // 构造失败时的 fail-closed 兜底（空表）：谁都不许。
    let deny_all = PermissionMatrix::from_names(&[]).expect("空表合法");
    assert!(authorize_close(&deny_all, crate::authz::PRINCIPAL_MAIN, true).is_err());
}

/// 三段在桥里真的接上了（[04 §3.1 批⑦](../../../../docs/plan/04-工程落地.md) 的接入
/// 判据）：每轮编码一条（步 11）、第 4 条到齐自动巩固（步 13）、收束时把本轮检索
/// 落成事实（步 12）——全部在同一份 WAL 里可读，且 `check_all` 全绿（溯源 100%）。
#[test]
fn record_to_wal_wires_all_three_memory_steps() {
    let wal = tmp_wal("memory-wiring");

    // 轮 0–3：每轮说一句、四句互不相同 ⇒ 第 4 轮收束时活记忆到 4 条 ⇒ 巩固触发。
    for i in 0..4u64 {
        record_to_wal(
            wal.clone(),
            crate::authz::PRINCIPAL_MAIN,
            &format!("u-{i}"),
            &format!("第 {i} 条约定"),
            V2Closure::Final {
                text: "答".into(),
                cost_ms: i,
            },
            None,
            &[],
        )
        .expect("转写成功");
    }

    let store = EventWalStore::open(&wal).expect("打开 WAL");
    let snap = store.range(Seq::new(0));
    assert_eq!(
        snap.iter()
            .filter(|e| e.kind == EVENT_MEMORY_ENCODED)
            .count(),
        4,
        "步 11：每轮各编码一条"
    );
    snap.iter()
        .find(|e| e.kind == EVENT_MEMORY_CONSOLIDATED)
        .expect("步 13：第 4 条到齐即触发巩固")
        .payload
        .get("generation")
        .and_then(|v| v.as_u64())
        .filter(|g| *g == 1)
        .expect("首次合并代数 = 1");
    assert_eq!(
        snap.iter()
            .filter(|e| e.kind == EVENT_MEMORY_FORGOTTEN)
            .count(),
        2,
        "最旧的两条源记忆被排除式遗忘（Log 不删，只是投影不再包含）"
    );

    // 步 12：视图在收束前读出，收束时才落成事实（溯源锚那时才存在）。
    let view = recall(crate::authz::PRINCIPAL_USER, None)
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    assert!(!view.entries.is_empty(), "还有活记忆可召回");
    drop(store);

    record_to_wal(
        wal.clone(),
        crate::authz::PRINCIPAL_MAIN,
        "u-4",
        "第五句",
        V2Closure::Final {
            text: "答".into(),
            cost_ms: 44,
        },
        Some(&view),
        &[],
    )
    .expect("转写成功");

    let snap = EventWalStore::open(&wal).expect("重开").range(Seq::new(0));
    let recalled = snap
        .iter()
        .find(|e| e.kind == EVENT_MEMORY_RECALLED)
        .expect("步 12：检索事实入格");
    let fifth_user = snap
        .iter()
        .rfind(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("本轮用户格");
    assert_eq!(
        recalled.produced_by,
        fifth_user.seq().map(|s| s.value()),
        "溯源锚 = 触发检索的那格用户发言"
    );
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 代际立约（[04 §3.1 批⑧](../../../../docs/plan/04-工程落地.md)，S08 §3「加格子，不加机制」）：
/// 本轮的 `agent_run` 委托随收束落成 `commitment.opened` → `released`，
/// **同锚在本轮 `user.message`**——承诺是「这轮我答应了什么」，锚漂到别处就查不到了。
#[test]
fn settled_delegation_becomes_commitment_opened_then_released() {
    use super::super::tools::Delegation;

    let wal = tmp_wal("commit-ok");
    let delegations = [Delegation {
        id: "call-1".to_string(),
        promise: "让 reviewer 复查这段".to_string(),
        ok: true,
        why: String::new(),
    }];
    record_to_wal(
        wal.clone(),
        crate::authz::PRINCIPAL_MAIN,
        "u-9",
        "交给子智能体",
        V2Closure::Final {
            text: "已交给子智能体".into(),
            cost_ms: 50,
        },
        None,
        &delegations,
    )
    .expect("转写成功");

    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    let user = snap
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("用户格");
    let user_seq = user.seq().map(|s| s.value()).expect("已入格");
    let opened = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_COMMITMENT_OFFERED)
        .expect("立约格");
    let released = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_COMMITMENT_RELEASED)
        .expect("守约格");

    assert_eq!(opened.entity, Entity::Commitment);
    assert_eq!(
        opened.produced_by,
        Some(user_seq),
        "立约锚在本轮开口上（溯源可查）"
    );
    assert_eq!(
        released.produced_by,
        Some(user_seq),
        "立约与了结同锚成对——`check_all` 才看得见它们是一对"
    );
    assert_eq!(
        released.payload.get("id"),
        opened.payload.get("id"),
        "了结按载荷 id 找回立约方（收束时不重抄 `from`）"
    );
    assert_eq!(
        opened.payload.get("from").and_then(|v| v.as_str()),
        Some(crate::authz::PRINCIPAL_MAIN),
        "承诺方 = 会话主体（声誉记在承诺方头上）"
    );
    assert_eq!(
        opened.payload.get("to").and_then(|v| v.as_str()),
        Some(crate::authz::PRINCIPAL_USER),
        "承诺对象 = 会话外的另一方"
    );
    assert_eq!(
        opened.payload.get("promise").and_then(|v| v.as_str()),
        Some("让 reviewer 复查这段"),
        "承诺内容 = 委托出去的那句话"
    );
    // 承诺号带 `{user_id}-a{attempt}` 前缀：调用编号只在**一次模型响应内**唯一，
    // 而 WAL 的幂等键是事件 id——不加前缀，跨轮复用同一编号会让第二次立约撞
    // `Duplicate`、把整轮转写拖失败。
    assert!(
        opened.event_id.starts_with("c-offer-v2c-u-9-a0-"),
        "承诺号须带轮次前缀：{}",
        opened.event_id
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 违约必须可观测（S08 §5）：`broken` 带 `why`，并**宣告**给承诺对象——
/// 宣告仍是同一份事实源里的一格，不是新通道（S08 §2「通信 = 没有直连」）。
#[test]
fn breached_delegation_is_declared_to_the_counterparty() {
    use super::super::tools::Delegation;

    let wal = tmp_wal("commit-breach");
    let delegations = [Delegation {
        id: "call-2".to_string(),
        promise: "让 reviewer 复查这段".to_string(),
        ok: false,
        why: "子会话中途失败".to_string(),
    }];
    record_to_wal(
        wal.clone(),
        crate::authz::PRINCIPAL_MAIN,
        "u-10",
        "交给子智能体",
        V2Closure::Final {
            text: "没交出去".into(),
            cost_ms: 50,
        },
        None,
        &delegations,
    )
    .expect("转写成功");

    let snap = EventWalStore::open(&wal).expect("重开").range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    let broken = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_COMMITMENT_BROKEN)
        .expect("违约收束必须成格");
    assert_eq!(
        broken.payload.get("why").and_then(|v| v.as_str()),
        Some("子会话中途失败"),
        "违约必须带 why（可观测，S08 §5）"
    );

    let asserted = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_COMMITMENT_ASSERTED)
        .expect("违约宣告必须成格");
    assert_eq!(asserted.entity, Entity::Commitment);
    let statement = asserted
        .payload
        .get("statement")
        .and_then(|v| v.as_str())
        .expect("宣告载荷带原话");
    assert!(
        statement.contains(crate::authz::PRINCIPAL_USER) && statement.contains("子会话中途失败"),
        "宣告 = 把这次违约告知承诺对象：{statement}"
    );
    let user = snap
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("用户格");
    assert_eq!(
        asserted.produced_by,
        user.seq().map(|s| s.value()),
        "宣告同样锚在本轮开口上"
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}
