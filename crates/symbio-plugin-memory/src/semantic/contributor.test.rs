//! 语义记忆贡献的判据：三个读数逐字誊抄、锚指向本轮开口、同锚不重复、不变量不破。
//!
//! 这五条原来钉在 `loop` 的 `memory_events.test.rs`（写方在那边时）。写方搬进本包，
//! 判据跟着搬——**不是删掉重写**，是同一批断言换了个能同时看见读数与事实的地方。

use super::*;
use symbio_plugin_api::events::EVENT_USER_MESSAGE;
use symbio_plugin_api::{
    check_all, vocab, Entity, Event, EventStore, RecallLine, Seq, SettleFace, Store, Verb,
    PRINCIPAL_USER,
};

/// 一份读数：`(seq, content, tag)` 列表 + 降级位。
fn facts<'a>(lines: &'a [RecallLine<'a>], degraded: bool) -> RecallFacts<'a> {
    RecallFacts { degraded, lines }
}

/// 收束这一刻的值。只有 `recall` / `user_seq` / `history` 与本模块有关，其余取平凡值。
fn settling<'a>(
    recall: Option<&'a RecallFacts<'a>>,
    user_seq: u64,
    history: &'a [Event],
) -> Settling<'a> {
    Settling {
        face: SettleFace::Generated,
        now_ms: 9_000,
        principal: PRINCIPAL_USER,
        user_seq,
        anchor: "t1",
        user_text: "记住我的幸运数字",
        response: Some("好"),
        recall,
        history,
    }
}

/// 三个读数全部来自读数本身：`found` = 条目数、`top` = 首条正文、`degraded` 直誊。
#[test]
fn the_three_readings_are_copied_from_the_retrieval() {
    let lines = [
        RecallLine {
            seq: 7,
            content: "用户偏好简洁答复",
            tag: "经验",
        },
        RecallLine {
            seq: 3,
            content: "项目用 Rust 2021",
            tag: "经验",
        },
    ];
    let recall = facts(&lines, false);
    let at = settling(Some(&recall), 42, &[]);

    let out = SemanticRecall.contribute(&at);
    assert_eq!(out.len(), 1, "有读数 ⇒ 一条事实");
    let fact = &out[0].fact;
    assert_eq!(fact.payload["found"], 2, "found 是摘除之后的条目数");
    assert_eq!(fact.payload["top"], "用户偏好简洁答复", "top 是首条正文");
    assert_eq!(fact.payload["degraded"], false, "不降级不标降级");
}

/// 降级必须落进事实：它是**生产读数口**，只在端口 / 提示词里活着就等于没有。
#[test]
fn degraded_is_recorded_instead_of_dropped() {
    let lines = [RecallLine {
        seq: 0,
        content: "片段 0",
        tag: "经验",
    }];
    let partial = facts(&lines, true);
    let out = SemanticRecall.contribute(&settling(Some(&partial), 1, &[]));
    assert_eq!(
        out[0].fact.payload["degraded"], true,
        "部分结果的检索必须把 degraded 写进事实——读数口靠的是这条事件"
    );

    let full = facts(&lines, false);
    let out = SemanticRecall.contribute(&settling(Some(&full), 1, &[]));
    assert_eq!(out[0].fact.payload["degraded"], false, "不谎称降级");
}

/// 本轮没走检索 ⇒ 不落笔（空是合法结局，不是失败）。
#[test]
fn no_retrieval_means_no_fact() {
    assert!(
        SemanticRecall
            .contribute(&settling(None, 1, &[]))
            .is_empty(),
        "答话轮 / 无活记忆：`Settling.recall` 为 None ⇒ 没有可誊抄的读数"
    );
}

/// 锚 = 触发检索的那格用户发言（I2），幂等键 `recalled-{user_seq}` 与它同一格。
#[test]
fn the_anchor_points_at_the_turn_opening() {
    let lines = [RecallLine {
        seq: 0,
        content: "一条",
        tag: "经验",
    }];
    let recall = facts(&lines, false);
    let out = SemanticRecall.contribute(&settling(Some(&recall), 77, &[]));
    assert_eq!(
        out[0].fact.produced_by,
        Some(77),
        "断言类必带溯源：锚指向触发检索的用户格"
    );
    assert_eq!(
        out[0].fact.key, "recalled-77",
        "幂等键的词干由宿主算（D-252），能力只拼 user_seq"
    );
}

/// 同一轮被重复收束 ⇒ 不落第二条（重入的守卫读 `history`，不读 store 句柄）。
#[test]
fn a_second_settle_of_the_same_turn_does_not_duplicate() {
    let store = EventStore::new();
    let lines = [RecallLine {
        seq: 0,
        content: "一条",
        tag: "经验",
    }];
    let recall = facts(&lines, false);
    let first = SemanticRecall.contribute(&settling(Some(&recall), 5, &[]));
    assert_eq!(first.len(), 1);

    // 落格（宿主的那一步：信封由它盖）。
    let mut event = first[0].fact.clone().into_event(PRINCIPAL_USER);
    event.ts = 9_000;
    store.append(event).expect("入格");

    let history = store.range(Seq::new(0));
    let again = SemanticRecall.contribute(&settling(Some(&recall), 5, &history));
    assert!(
        again.is_empty(),
        "同锚已落 ⇒ 跳过：重复收束不得给同一轮开口制造第二条召回事实"
    );
}

/// 全链路：这条事实落格后，网格的不变量仍全绿（I2 的溯源覆盖是其中之一）。
#[test]
fn the_fact_keeps_the_invariants_green() {
    let store = EventStore::new();
    // 根事件先落：召回事实的溯源指向它，否则那条边指向自己。
    store
        .append(Event::pending(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Scope,
            Verb::Opened,
            PRINCIPAL_USER,
        ))
        .expect("用户格入格");
    let user_seq = store
        .range(Seq::new(0))
        .first()
        .and_then(|e| e.seq)
        .map(|s| s.value())
        .expect("用户格有 seq");

    let lines = [RecallLine {
        seq: 0,
        content: "用户偏好简洁答复",
        tag: "经验",
    }];
    let recall = facts(&lines, false);
    let out = SemanticRecall.contribute(&settling(Some(&recall), user_seq, &[]));
    let mut event = out[0].fact.clone().into_event(PRINCIPAL_USER);
    event.ts = 9_000;
    store.append(event).expect("入格");

    let snapshot = store.range(Seq::new(0));
    assert!(
        check_all(vocab(), &snapshot).is_empty(),
        "{:?}",
        check_all(vocab(), &snapshot)
    );
    let recalled = snapshot
        .iter()
        .find(|e| e.kind == EVENT_MEMORY_RECALLED)
        .expect("召回事实在事实源里");
    assert_eq!(recalled.payload["found"], 1);
}
