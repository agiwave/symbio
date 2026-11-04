//! `loop.step` 事实的单元用例。
//!
//! 两条主线：
//!
//! 1. **格的形状**——事件 id、载荷、落格元数据与重复落格的吸收；
//! 2. **D-107 判据 2 的事实面**——「这一步做过」按 `(turn, 幕锚, id)` 折叠；
//!    按幕锚的收窄在 `turn_head` 视图（续写幕复用轮号时不得把首幕的格算进来）。
//!
//! 行在这一格上关不关账由 `loop_rows` 的 `until` 负责，用例在那边。

use super::{done, land, source_of, StageFact, VerdictFact, VerdictSource};
use symbio_kernel::dialog::{
    Verdict, REASON_ACK, REASON_CLARIFY, REASON_EMPTY, REASON_FROM_CONTEXT, REASON_GREETING,
    REASON_NEEDS_WORK, REASON_REFUSE, REASON_THANKS, REASON_UNCLASSIFIED,
};
use symbio_kernel::event::{Entity, Seq, Verb};
use symbio_kernel::store::{EventStore, Store};

use crate::control_decl::EVENT_LOOP_STEP;
use crate::loop_rows::{STEP_COMPRESS, STEP_DECIDE, STEP_INPUTS, STEP_RECALL};

// ── 来源划分：词表每一条都要有归属 ─────────────────────────────────────

#[test]
fn every_reason_has_a_source() {
    // 加一个理由码而不更新映射 ⇒ 这里红。词表是数据，映射是它的分区；
    // 分区漏一条时来源统计会静默说谎（`source_of` 的兜底恰好也是 `Fallback`，
    // 所以只测「有没有值」不够，必须逐条钉到具体来源）。
    let cases = [
        (REASON_GREETING, VerdictSource::Rule),
        (REASON_THANKS, VerdictSource::Rule),
        (REASON_ACK, VerdictSource::Rule),
        (REASON_EMPTY, VerdictSource::Rule),
        (REASON_FROM_CONTEXT, VerdictSource::Model),
        (REASON_CLARIFY, VerdictSource::Model),
        (REASON_REFUSE, VerdictSource::Model),
        (REASON_NEEDS_WORK, VerdictSource::Model),
        (REASON_UNCLASSIFIED, VerdictSource::Fallback),
    ];
    for (reason, want) in cases {
        assert_eq!(source_of(reason), want, "理由码 {reason} 的来源");
    }
    // 未登记的理由码按兜底记——既不是规则也不是模型，两侧都会说谎。
    assert_eq!(source_of("future_reason"), VerdictSource::Fallback);
}

#[test]
fn classify_fact_carries_reason_and_source() {
    let ruled = Verdict::Answered {
        reason: REASON_GREETING.into(),
    };
    assert_eq!(
        StageFact::classify(STEP_DECIDE, 0, Some(&ruled), 0),
        StageFact {
            id: STEP_DECIDE,
            round: 0,
            verdict: Some(VerdictFact {
                reason: Some(REASON_GREETING.into()),
                source: VerdictSource::Rule,
            }),
            result: None,
            identity: None,
            model_calls: 0,
            recalled: Vec::new(),
            resumed: false,
            replaces: None,
            recalled_content: Vec::new(),
            degraded: false,
            answer: None,
            opening: None,
        }
    );

    let modelled = Verdict::Escalate {
        reason: REASON_NEEDS_WORK.into(),
    };
    assert_eq!(
        StageFact::classify(STEP_DECIDE, 4, Some(&modelled), 1).verdict,
        Some(VerdictFact {
            reason: Some(REASON_NEEDS_WORK.into()),
            source: VerdictSource::Model,
        })
    );

    // 分类器没给出判决（未挂载 / 路由失败）⇒ 仍是一次执行，记 `absent`。
    assert_eq!(
        StageFact::classify(STEP_DECIDE, 0, None, 0),
        StageFact {
            id: STEP_DECIDE,
            round: 0,
            verdict: Some(VerdictFact {
                reason: None,
                source: VerdictSource::Absent,
            }),
            result: None,
            identity: None,
            model_calls: 0,
            recalled: Vec::new(),
            resumed: false,
            replaces: None,
            recalled_content: Vec::new(),
            degraded: false,
            answer: None,
            opening: None,
        }
    );
}

#[test]
fn payload_carries_only_what_the_fact_has() {
    // 没有判决的一步：载荷只有 id + 幕锚 + 轮次，不带 `source` / `reason`。
    assert_eq!(
        StageFact::ran(STEP_INPUTS, 2).payload(4),
        serde_json::json!({ "id": "inputs", "anchor": 4, "round": 2 })
    );
    // 召回例外：它的载荷还带逐条结果与 `resumed`（`turn.recalled` 的数据面）。
    assert_eq!(
        StageFact::recall(
            STEP_RECALL,
            2,
            9,
            &[
                (3, "经验".into(), "正文一".into()),
                (7, "skill".into(), "正文二".into()),
            ],
            false,
            false,
        )
        .payload(4),
        serde_json::json!({
            "id": "recall",
            "anchor": 4,
            "round": 2,
            "recalled": [
                { "seq": 3, "tag": "经验", "content": "正文一" },
                { "seq": 7, "tag": "skill", "content": "正文二" }
            ],
            "resumed": false,
            "degraded": false,
        })
    );

    let decided = StageFact::classify(
        STEP_DECIDE,
        0,
        Some(&Verdict::Answered {
            reason: REASON_GREETING.into(),
        }),
        0,
    );
    assert_eq!(
        decided.payload(4),
        serde_json::json!({
            "id": "decide",
            "anchor": 4,
            "round": 0,
            "source": "rule",
            "reason": REASON_GREETING,
            "model_calls": 0,
        })
    );

    // 没有理由码时 `reason` 整键缺席，而不是 `null`：读侧按 `as_str` 取值，
    // 两种写法虽都能得到 `None`，但「没有这个事实」与「有事实且为空」不该混。
    assert_eq!(
        StageFact::classify(STEP_DECIDE, 0, None, 0).payload(4),
        serde_json::json!({ "id": "decide", "anchor": 4, "round": 0, "source": "absent", "model_calls": 0 })
    );
}

#[test]
fn the_model_call_count_is_carried_by_the_decide_fact() {
    // `source` 读不出「有没有打过模型」（模型调了而应答不可解析 ⇒ `source=fallback`），
    // 那次往返在事实层唯一的**数**是 `model_calls`：判据侧拿它对账「观测 ⇔ 三格」。
    let modelled = Verdict::Escalate {
        reason: REASON_NEEDS_WORK.into(),
    };
    assert_eq!(
        StageFact::classify(STEP_DECIDE, 0, Some(&modelled), 1).payload(4)["model_calls"],
        serde_json::json!(1)
    );
    // 其余步不写这个字段——「恒 0 的字段」是噪声。
    assert_eq!(
        StageFact::ran(STEP_RECALL, 0).payload(4).get("model_calls"),
        None
    );
}

// ── 同一性与落格 ───────────────────────────────────────────────────────

#[test]
fn the_compress_fact_carries_its_call_count_and_a_per_act_identity() {
    // 压缩在续写幕会重跑（同一轮号、`request_round` 从 0 重开）。同一性若只按
    // `(turn, id)` 算，续写幕那格会撞首幕的幂等键 ⇒ 载荷（含
    // `model_calls`）**不落** ⇒ 那一笔模型往返在事实层没了数，判据侧对不上账。
    // 所以 identity = 本幕的锚（`drive_seq`）：两幕各一格。
    let first = StageFact::compress(STEP_COMPRESS, 0, 2, 17);
    assert_eq!(first.event_id(7, 17), "st-7-compress-17");
    assert_eq!(
        first.payload(17)["model_calls"],
        serde_json::json!(2),
        "压缩那格必须带着这次往返的数"
    );
    assert_ne!(
        first.event_id(7, 17),
        StageFact::compress(STEP_COMPRESS, 0, 1, 23).event_id(7, 23),
        "两幕的压缩是两格；撞键会让后一幕的 `model_calls` 静默消失"
    );
    // `identity` 只进事件 id，不进载荷——格里的数是 `model_calls`，不是锚。
    assert!(first.payload(17).get("identity").is_none());
}

#[test]
fn event_id_is_the_step_identity() {
    assert_eq!(
        StageFact::ran(STEP_DECIDE, 0).event_id(7, 0),
        "st-7-decide-0"
    );
    assert_eq!(
        StageFact::classify(STEP_DECIDE, 2, None, 0).event_id(7, 2),
        "st-7-decide-2"
    );
    assert_ne!(
        StageFact::ran(STEP_RECALL, 0).event_id(7, 0),
        StageFact::ran(STEP_COMPRESS, 0).event_id(7, 0),
        "同一幕的两个步必须是两个幂等键，否则后落的那格会被当成重复丢掉"
    );
    assert_ne!(
        StageFact::ran(STEP_RECALL, 0).event_id(7, 0),
        StageFact::ran(STEP_RECALL, 0).event_id(7, 1),
        "跨幕的重跑是新的一次执行（轮号不进 id，幕锚进 id）"
    );
}

#[test]
fn land_writes_the_loop_step_cell() {
    let store = EventStore::new();
    let fact = StageFact::classify(
        STEP_DECIDE,
        0,
        Some(&Verdict::Escalate {
            reason: REASON_NEEDS_WORK.into(),
        }),
        1,
    );
    land(&store, 7, 4, "user-1", &[fact]).expect("落格失败");

    let events = store.range(Seq::new(0));
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.event_id, "st-7-decide-4");
    assert_eq!(e.kind, EVENT_LOOP_STEP);
    assert_eq!(e.entity, Entity::Scope);
    assert_eq!(e.verb, Verb::Asserted);
    assert_eq!(e.scope, 7);
    assert_eq!(e.actor, "user-1");
    // I2：断言类事件必须带溯源（幕锚 seq）。
    assert_eq!(e.produced_by, Some(4));
    assert_eq!(e.payload["id"], "decide");
    assert_eq!(e.payload["anchor"], 4);
    assert_eq!(e.payload["round"], 0);
    assert_eq!(e.payload["source"], "model");
    assert_eq!(e.payload["reason"], REASON_NEEDS_WORK);
}

#[test]
fn landing_the_same_stage_twice_is_absorbed() {
    // 续写幕复用同一轮号且 `request_round` 从 0 重开，召回 / 压缩会重跑；
    // 重跑撞幂等键不是失败——这一步确实已经做过。
    let store = EventStore::new();
    let fact = StageFact::ran(STEP_RECALL, 0);
    let one = std::slice::from_ref(&fact);
    land(&store, 7, 0, "user", one).expect("首次落格失败");
    land(&store, 7, 0, "user", one).expect("重复落格应被吸收，而不是让整轮失败");
    assert_eq!(store.len(), 1, "重复落格不得产生第二格");
}

// ── `done` 投影：范围恰好是 (turn, id, round) ──────────────────────────

#[test]
fn done_is_scoped_by_turn_step_and_round() {
    let store = EventStore::new();
    land(&store, 7, 0, "user", &[StageFact::ran(STEP_RECALL, 0)]).unwrap();
    land(&store, 7, 0, "user", &[StageFact::ran(STEP_COMPRESS, 0)]).unwrap();
    let events = store.range(Seq::new(0));

    assert!(done(&events, 7, STEP_RECALL, 0));
    assert!(
        !done(&events, 8, STEP_RECALL, 0),
        "别轮的同一阶段不算本轮已做"
    );
    assert!(!done(&events, 7, STEP_RECALL, 1), "别轮次不算本请求轮已做");
    assert!(
        !done(&events, 7, STEP_DECIDE, 0),
        "同轮另一阶段没有落格，不得互相顶替"
    );
}
