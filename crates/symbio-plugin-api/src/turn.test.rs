//! 轮末贡献的判据：次序按 `(prio, owner)`、归属可声明、时刻由宿主盖章。

use super::*;
use symbio_core::{Entity, Verb};

fn draft(key: &str) -> FactDraft {
    FactDraft {
        key: key.to_string(),
        kind: "test.contributed".to_string(),
        entity: Entity::Domain,
        verb: Verb::Asserted,
        payload: serde_json::json!({ "n": 1 }),
        produced_by: Some(7),
    }
}

/// 两个只为测试存在的贡献者：次序靠 `prio` 与 owner 两格共同决定。
struct Early;

impl TurnContributor for Early {
    fn owner(&self) -> &'static str {
        "tests/alpha"
    }

    fn prio(&self) -> u8 {
        10
    }

    fn contribute(&self, end: &TurnEnd<'_>) -> Vec<TurnDraft> {
        vec![TurnDraft::new(draft(&format!("{}-a", end.anchor())))]
    }
}

struct Late;

impl TurnContributor for Late {
    fn owner(&self) -> &'static str {
        "tests/omega"
    }

    fn prio(&self) -> u8 {
        90
    }

    fn contribute(&self, end: &TurnEnd<'_>) -> Vec<TurnDraft> {
        vec![TurnDraft::new(draft(&format!("{}-z", end.anchor())))]
    }
}

// 判据用**进程内**的提交：本 crate 的单测里没有别的插件，`inventory` 里只有这几条。
crate::submit_turn_contributor!(Late);
crate::submit_turn_contributor!(Early);

fn end() -> TurnEnd<'static> {
    TurnEnd {
        face: TurnFace::Generated,
        turn: 3,
        now_ms: 1_700,
        principal: "session/a",
        user_seq: 42,
        anchor: "t3",
        user_text: "说了什么",
        response: Some("答了什么"),
        task_cap: 8,
        history: &[],
    }
}

/// `prio` 小者先；同 `prio` 时 owner 字典序（与效果 / 行为端口同一条装配纪律）。
#[test]
fn contributors_are_ordered_by_prio_then_owner() {
    let owners: Vec<&str> = contributors().iter().map(|c| c.owner()).collect();
    assert_eq!(owners, ["tests/alpha", "tests/omega"]);
}

/// 草稿的轮号与时刻由宿主盖章，归属缺省是本轮主体。
#[test]
fn draft_is_stamped_by_the_host() {
    let end = end();
    let drafted = TurnDraft::new(draft("d-1"));
    assert_eq!(drafted.actor, None);
    let event = drafted.into_event(&end);
    assert_eq!(event.turn, 3);
    assert_eq!(event.ts, 1_700);
    assert_eq!(event.actor, "session/a");
    assert_eq!(event.event_id, "d-1");
    assert_eq!(event.kind, "test.contributed");
    assert_eq!(event.produced_by, Some(7));
}

/// 能力可以声明归属人（记忆的事实归人，不归跑轮主体）；轮号与时刻不受影响。
#[test]
fn actor_can_be_declared_by_the_capability() {
    let end = end();
    let event = TurnDraft::new(draft("d-2"))
        .with_actor("person/local")
        .into_event(&end);
    assert_eq!(event.actor, "person/local");
    assert_eq!(event.turn, 3);
    assert_eq!(event.ts, 1_700);
}

/// 缺省贡献是空：不关心轮末的能力不必实现 `contribute`。
#[test]
fn default_contribution_is_empty() {
    struct Silent;
    impl TurnContributor for Silent {
        fn owner(&self) -> &'static str {
            "tests/silent"
        }
    }
    assert!(Silent.contribute(&end()).is_empty());
}

/// 贡献者拿到的是纯值：入参没有 store、没有句柄，也不能改写本轮状态。
#[test]
fn contribute_takes_pure_values() {
    let end = end();
    assert_eq!(end.face, TurnFace::Generated);
    assert_eq!(end.anchor(), "t3");
    assert_eq!(end.user_seq, 42);
    assert_eq!(end.task_cap, 8);
    assert_eq!(end.response, Some("答了什么"));
    let all = contributors();
    let early = all.iter().find(|c| c.owner() == "tests/alpha").unwrap();
    let drafts = early.contribute(&end);
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].fact.key, "t3-a");
}
