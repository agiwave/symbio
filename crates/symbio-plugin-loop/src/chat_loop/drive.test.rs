//! 开一幕：用户格 / 配置 / 轮次事实先于任何判决入格。
//!
//! 三条判据各自钉一个输入闭包成员：
//!
//! - 新轮：`u-{turn}` 先落，轮号接着**水位**走，`session.config` 带当轮生效值；
//! - 续写幕：不新开用户格（同一句话只出现一次），恢复格是本幕锚，`cfg` 的
//!   幂等键带幕号（同一轮可以先后恢复两次）；
//! - 连开两轮：轮号接着已有用户格数走，`u-{turn}` 不重号。

use std::sync::Arc;

use super::super::super::test_support::{
    default_settings, loop_session, turn_spec, FakeSessionPort,
};
use super::super::state::{SessionContext, TurnRequest, TurnState};
use super::begin_drive;
use symbio_core::chat_node::{ChatNode, MessageContent, NodeKind};
use symbio_core::principal::{Principal, PrincipalKind};
use symbio_core::{
    count_user_messages, Entity, Event, EventStore, Seq, Store, Verb, EVENT_SESSION_CONSOLIDATED,
    EVENT_USER_MESSAGE,
};

use crate::control_decl::{EVENT_SESSION_CONFIG, EVENT_TURN_STAGE};

fn user_node(id: &str, text: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        principal: Some(Principal::new(PrincipalKind::Person, "user")),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 会话上下文：内存事实轨 + 一条用户消息 + 默认配置。
fn context(messages: Vec<ChatNode>) -> (Arc<EventStore>, SessionContext) {
    let facts = Arc::new(EventStore::new());
    let port = Arc::new(FakeSessionPort::new(messages.clone()).with_facts(facts.clone()));
    let spec = turn_spec(
        "s-drive",
        std::env::temp_dir(),
        std::env::temp_dir(),
        default_settings(),
    );
    let session = Arc::new(loop_session(spec, facts.clone(), port));
    (
        facts,
        SessionContext {
            messages,
            session,
            consolidated: None,
        },
    )
}

fn request() -> TurnRequest {
    TurnRequest {
        // 请求级覆盖：配置事实记的是**生效值**，不是配置默认值。
        auto_compress: false,
        load_history: true,
        system_prompt: None,
    }
}

fn fact<'a>(events: &'a [Event], kind: &str, turn: u64) -> &'a Event {
    events
        .iter()
        .find(|e| e.kind == kind && e.turn == turn)
        .unwrap_or_else(|| panic!("缺 {kind}（turn={turn}）：{events:?}"))
}

/// 新轮：用户格先落、配置随后、轮次事实最后；三者的锚都指向本幕。
#[test]
fn a_new_drive_lands_user_config_and_round_before_any_decision() {
    let (facts, context) = context(vec![user_node("m1", "你好")]);
    let mut turn = TurnState::default();

    let drive = begin_drive(&context, &mut turn, &request(), None).expect("新轮开得了");

    assert_eq!(turn.turn_no, 0);
    assert_eq!(drive.user_text, "你好");
    assert_eq!(turn.drive_seq, turn.user_seq, "新轮的幕锚就是用户格 seq");

    let all = facts.range(Seq::new(0));
    assert_eq!(all[0].kind, EVENT_USER_MESSAGE, "用户格是第一条事实");
    assert_eq!(all[0].event_id, "u-0");
    assert_eq!(all[0].payload["text"], "你好");
    assert_eq!(all[1].kind, EVENT_SESSION_CONFIG, "配置紧随用户格");

    let config = fact(&all, EVENT_SESSION_CONFIG, 0);
    assert_eq!(
        config.event_id,
        format!("cfg-0-{}", turn.drive_seq),
        "配置格的幂等键带幕号（同一轮可以有多幕）"
    );
    assert_eq!(config.produced_by, Some(turn.user_seq));
    assert_eq!(
        config.payload["classify_enabled"], true,
        "配置事实记当轮生效值"
    );
    assert_eq!(
        config.payload["auto_compress"], false,
        "请求级覆盖已折进生效值（不是配置默认值）"
    );
    assert_eq!(config.payload["is_persistent"], true);

    let round = fact(&all, EVENT_TURN_STAGE, 0);
    assert_eq!(round.payload["stage"], "round");
    assert_eq!(round.payload["round"], 0);
    assert_eq!(round.produced_by, Some(turn.user_seq));

    assert_eq!(turn.turn_no, 0);
    assert_eq!(turn.user_seq, turn.user_seq);
    assert_eq!(turn.drive_seq, turn.drive_seq);
    assert_eq!(turn.facts.len(), all.len(), "本幕读到的事实切片已刷新");
}

/// 轮号接**水位**：快照取代了 1 轮之后，新轮落 `u-1` 而不是从 0 重号。
#[test]
fn the_turn_number_continues_across_a_snapshot() {
    let (facts, context) = context(vec![user_node("m1", "第二问")]);
    facts
        .append(
            Event::pending(
                "u-0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "第一问" })),
        )
        .expect("第一轮用户格");
    facts
        .append(
            Event::pending(
                "v2sc-0",
                EVENT_SESSION_CONSOLIDATED,
                Entity::Message,
                Verb::Progressed,
                0,
                "session/main",
            )
            .with_payload(serde_json::json!({ "messages": [], "turns": 1 })),
        )
        .expect("快照");

    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), None).expect("新轮开得了");

    assert_eq!(turn.turn_no, 1, "轮号接着水位走");
    assert!(
        facts
            .range(Seq::new(0))
            .iter()
            .any(|e| e.event_id == format!("st-1-0-round-{}", turn.drive_seq)),
        "轮次事实的幂等键带幕号"
    );
    assert!(
        facts.range(Seq::new(0)).iter().any(|e| e.event_id == "u-1"),
        "第二轮用户格是 u-1（不重号）"
    );
}

/// 续写幕：不新开用户格；恢复格是本幕锚，`cfg` 的键带幕号。
#[test]
fn a_resume_drive_reuses_the_open_turn_without_a_new_user_fact() {
    let (facts, context) = context(vec![user_node("m1", "请批准")]);
    facts
        .append(
            Event::pending(
                "u-0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "请批准" })),
        )
        .expect("等待轮的用户格");

    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), Some("approve")).expect("续写幕开得了");

    assert_eq!(turn.turn_no, 0, "续写记在原轮上");
    assert_eq!(
        facts
            .range(Seq::new(0))
            .iter()
            .filter(|e| e.kind == EVENT_USER_MESSAGE)
            .count(),
        1,
        "续写不重开用户格"
    );

    let all = facts.range(Seq::new(0));
    let resume = fact(&all, EVENT_TURN_STAGE, 0);
    assert_eq!(resume.payload["stage"], "resume");
    assert_eq!(resume.payload["result"], "approve");
    assert_eq!(resume.produced_by, Some(turn.user_seq));
    assert_eq!(
        turn.drive_seq,
        resume.seq.map(|s| s.value()).unwrap(),
        "续写幕的锚是恢复事实的 seq（不是原用户格）"
    );
    assert_eq!(
        fact(&all, EVENT_SESSION_CONFIG, 0).event_id,
        format!("cfg-0-{}", turn.drive_seq),
        "同一轮的配置格按幕区分（续写可能带不同的请求级配置）"
    );
}

/// 同一轮先后恢复两次：恢复格的序数进幂等键，第二次不被静默吸收。
#[test]
fn a_second_resume_in_the_same_turn_gets_its_own_anchor() {
    let (facts, context) = context(vec![user_node("m1", "请批准")]);
    facts
        .append(
            Event::pending(
                "u-0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "请批准" })),
        )
        .expect("等待轮的用户格");

    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), Some("approve")).expect("第一幕");
    let first_anchor = turn.drive_seq;
    begin_drive(&context, &mut turn, &request(), Some("answer")).expect("第二幕");
    let second_anchor = turn.drive_seq;

    assert_ne!(first_anchor, second_anchor, "两幕的锚不同");
    let all = facts.range(Seq::new(0));
    let resumes: Vec<&Event> = all
        .iter()
        .filter(|e| e.kind == EVENT_TURN_STAGE && e.payload["stage"] == "resume")
        .collect();
    assert_eq!(resumes.len(), 2, "两次恢复各落一格：{all:?}");
    assert_eq!(resumes[0].payload["result"], "approve");
    assert_eq!(resumes[1].payload["result"], "answer");
    assert!(
        all.iter().any(|e| e.event_id == "st-0-0-resume-answer-2"),
        "第二次的序数进幂等键：{all:?}"
    );
    assert!(
        all.iter()
            .any(|e| e.event_id == format!("cfg-0-{second_anchor}")),
        "第二幕的配置格用自己的锚"
    );
}

/// 同一轮之后再来一次「新轮」：轮号接着走，不重号（幂等键不撞）。
#[test]
fn a_second_new_drive_gets_the_next_turn_number() {
    let (facts, context) = context(vec![user_node("m1", "第二问")]);
    facts
        .append(
            Event::pending(
                "u-0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "第一问" })),
        )
        .expect("先开一轮");

    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), None).expect("第二轮开得了");
    assert_eq!(turn.turn_no, 1, "轮号接着已有用户格数走");
    assert_eq!(count_user_messages(&facts.range(Seq::new(0))), 2);
    assert!(
        facts.range(Seq::new(0)).iter().any(|e| e.event_id == "u-1"),
        "第二次开轮落 u-1，不撞 u-0"
    );
}

// ── 收租窗口的边界（DE-015 §2.6 第 7 条的边界段）────────────────────────
//
// 看护只看快照尾部 `LEASE_WINDOW` 条（`lease_tail`）。窗口外的未回 `sent` 被切掉，
// 因此不会被判死、只能由 `waiting` 行挂着——这是常量选值的**边界**，不是安全保证。
// 断言切点与长度，不抄数字：窗口值变，这两条跟着变；切点写反（取头部 / 少切一条）
// 立刻红。

/// 超窗时只留最新的一批：最早的那条被切掉，其余原样保序。
#[test]
fn the_lease_window_keeps_only_the_newest_facts() {
    let facts = Arc::new(EventStore::new());
    let total = super::LEASE_WINDOW + 1;
    for i in 0..total {
        facts
            .append(Event::pending(
                format!("f-{i}"),
                EVENT_TURN_STAGE,
                Entity::Turn,
                Verb::Asserted,
                0,
                "system",
            ))
            .expect("落格");
    }
    let all = facts.range(Seq::new(0));
    assert_eq!(all.len(), total, "夹具自己先超窗一条");

    let tail = super::lease_tail(&all);
    assert_eq!(tail.len(), super::LEASE_WINDOW, "窗口只留最新的一批");
    assert_eq!(
        tail.first().map(|e| e.event_id.as_str()),
        all.get(1).map(|e| e.event_id.as_str()),
        "切掉的是最早的那一条（下标 0），窗口从下标 1 起"
    );
    assert_eq!(
        tail.last().map(|e| e.event_id.as_str()),
        all.last().map(|e| e.event_id.as_str()),
        "最新的一条一定在窗口里（否则看护看不到刚发生的事）"
    );
}

/// 不超窗时窗口 = 全量（没有多余的截断，也没有越界 panic）。
#[test]
fn a_short_snapshot_is_handed_over_whole() {
    let facts = Arc::new(EventStore::new());
    for i in 0..8 {
        facts
            .append(Event::pending(
                format!("s-{i}"),
                EVENT_TURN_STAGE,
                Entity::Turn,
                Verb::Asserted,
                0,
                "system",
            ))
            .expect("落格");
    }
    let all = facts.range(Seq::new(0));
    assert_eq!(
        super::lease_tail(&all).len(),
        all.len(),
        "短快照整份交出（`saturating_sub` 不许把它切成空）"
    );
    assert!(super::lease_tail(&[]).is_empty(), "空快照不 panic");
}
