//! 召回行的判据（DE-041 §5 / D-241 / D-262）：命中零往返作答、低置信回退、
//! 续写幕不走近路、判不了只关账。

use serde_json::json;
use symbio_plugin_api::events::{
    EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_ENCODED, EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::rows::{RowHandler, RowRequest, RowStep};
use symbio_plugin_api::{Entity, Event, Seq, Verb};

use super::SkillRecall;
use crate::decl::{SKILL_ANSWERED, SKILL_RECALL, SKILL_TAG};

fn ev(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    let mut e = Event::pending(
        format!("e-{seq}"),
        kind,
        Entity::Turn,
        Verb::Asserted,
        0,
        "session/main",
    );
    e.seq = Some(Seq::new(seq));
    e.ts = 1;
    e.payload = payload;
    e
}

fn fast_path() -> Event {
    ev(1, EVENT_SESSION_CONFIG, json!({ "skill_fast_path": true }))
}

fn skill(seq: u64, skill_id: &str, trigger: &str, content: &str) -> Event {
    ev(
        seq,
        EVENT_MEMORY_ENCODED,
        json!({ "tag": SKILL_TAG, "skill_id": skill_id, "trigger": trigger, "content": content }),
    )
}

fn stage(seq: u64, resumed: bool) -> Event {
    ev(
        seq,
        "turn.stage",
        json!({ "stage": "recall", "resumed": resumed }),
    )
}

fn utterance(seq: u64, text: &str) -> Event {
    ev(
        seq,
        EVENT_USER_MESSAGE,
        json!({ "text": text, "id": "u-1" }),
    )
}

fn calibrated(seq: u64, skill_id: &str, fallback: bool) -> Event {
    ev(
        seq,
        EVENT_MEMORY_CALIBRATED,
        json!({ "skill_id": skill_id, "fallback": fallback }),
    )
}

fn key() -> Vec<(String, String)> {
    vec![
        ("recall".to_string(), "10".to_string()),
        ("stage".to_string(), "20".to_string()),
    ]
}

fn request<'a>(
    key: &'a [(String, String)],
    facts: &'a [Event],
    history: &'a [Event],
) -> RowRequest<'a> {
    RowRequest {
        row: SKILL_RECALL,
        key,
        anchor: "t1",
        now_ms: 2,
        principal: "session/main",
        user_seq: 30,
        facts,
        history,
    }
}

/// 触发串逐字相等 ⇒ 零往返作答：正文即技能正文，同一次判定落观测与判定两格。
#[test]
fn literal_trigger_answers_without_model_round() {
    let key = key();
    let facts = [utterance(29, "把周报整理成摘要")];
    let history = [
        fast_path(),
        skill(10, "sk-abc", "把周报整理成摘要", "先列要点；再合并同类项。"),
        stage(20, false),
    ];
    let RowStep::Answer(answer) = SkillRecall.run(&request(&key, &facts, &history)) else {
        panic!("触发串命中必须零往返作答");
    };
    assert_eq!(answer.text, "先列要点；再合并同类项。");
    assert_eq!(answer.facts.len(), 2, "观测与判定都落");
    let observed = &answer.facts[0];
    assert_eq!(observed.kind, EVENT_MEMORY_CALIBRATED);
    assert_eq!(observed.payload["skill_id"], json!("sk-abc"));
    assert_eq!(observed.payload["fallback"], json!(false));
    let judged = &answer.facts[1];
    assert_eq!(judged.kind, SKILL_ANSWERED);
    assert_eq!(judged.payload["recall"], json!(10));
    assert_eq!(judged.payload["stage"], json!(20));
    assert_eq!(judged.payload["hit"], json!(true));
    assert_eq!(judged.produced_by, Some(30), "判定带本轮开口格溯源");
}

/// 一次回退就把置信度打到阈值下 ⇒ 不回退视图之外再作答，只落观测与判定。
#[test]
fn low_confidence_skill_retreats_and_falls_back() {
    let key = key();
    let facts = [utterance(29, "把周报整理成摘要")];
    let history = [
        fast_path(),
        skill(10, "sk-abc", "把周报整理成摘要", "先列要点。"),
        stage(20, false),
        calibrated(40, "sk-abc", true),
    ];
    let RowStep::Facts(drafts) = SkillRecall.run(&request(&key, &facts, &history)) else {
        panic!("低置信技能不该作答");
    };
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].payload["fallback"], json!(true), "这次判定记回退");
    assert_eq!(drafts[1].payload["hit"], json!(false));
}

/// 续写幕不走快路：落格形状会撞幂等键，行只观测、不作答。
#[test]
fn resumed_stage_never_answers() {
    let key = key();
    let facts = [utterance(29, "把周报整理成摘要")];
    let history = [
        fast_path(),
        skill(10, "sk-abc", "把周报整理成摘要", "先列要点。"),
        stage(20, true),
    ];
    let RowStep::Facts(drafts) = SkillRecall.run(&request(&key, &facts, &history)) else {
        panic!("续写幕不该作答");
    };
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[1].payload["hit"], json!(false));
    assert_eq!(
        drafts[0].payload["fallback"],
        json!(false),
        "不命中不等于回退"
    );
}

/// 跨会话遗留 / 巩固产物缺 `skill_id` 与 `trigger` ⇒ 不观测、只关账。
#[test]
fn entries_without_skill_id_only_close_the_debt() {
    let key = key();
    let facts = [utterance(29, "把周报整理成摘要")];
    let history = [
        fast_path(),
        ev(
            10,
            EVENT_MEMORY_ENCODED,
            json!({ "tag": SKILL_TAG, "content": "旧经验" }),
        ),
        stage(20, false),
    ];
    let RowStep::Facts(drafts) = SkillRecall.run(&request(&key, &facts, &history)) else {
        panic!("判不了不该作答");
    };
    assert_eq!(drafts.len(), 1, "只落判定、不落观测");
    assert_eq!(drafts[0].kind, SKILL_ANSWERED);
    assert_eq!(drafts[0].payload["hit"], json!(false));
    assert_eq!(drafts[0].payload["skill_id"], json!(""));
}

/// 债键不完整（装配面出错）⇒ 交空集，由派发点的步数上界兜底，不造假的判定。
#[test]
fn missing_key_yields_no_facts() {
    let key = vec![("recall".to_string(), "10".to_string())];
    let RowStep::Facts(drafts) = SkillRecall.run(&request(&key, &[], &[])) else {
        panic!("缺键不该作答");
    };
    assert!(drafts.is_empty());
}
