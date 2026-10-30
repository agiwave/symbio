//! 收束贡献的判据：次序按 `(prio, owner)`、归属可声明、信封由宿主盖。

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

impl Contributor for Early {
    fn owner(&self) -> &'static str {
        "tests/alpha"
    }

    fn prio(&self) -> u8 {
        10
    }

    fn contribute(&self, at: &Settling<'_>) -> Vec<Contribution> {
        vec![Contribution::new(draft(&format!("{}-a", at.anchor)))]
    }
}

struct Late;

impl Contributor for Late {
    fn owner(&self) -> &'static str {
        "tests/omega"
    }

    fn prio(&self) -> u8 {
        90
    }

    fn contribute(&self, at: &Settling<'_>) -> Vec<Contribution> {
        vec![Contribution::new(draft(&format!("{}-z", at.anchor)))]
    }
}

// 判据用**进程内**的提交：本 crate 的单测里没有别的插件，`inventory` 里只有这几条。
crate::submit_contributor!(Late);
crate::submit_contributor!(Early);

fn at() -> Settling<'static> {
    Settling {
        face: SettleFace::Generated,
        now_ms: 1_700,
        principal: "session/a",
        user_seq: 42,
        anchor: "t3",
        user_text: "说了什么",
        response: Some("答了什么"),
        history: &[],
    }
}

/// `prio` 小者先；同 `prio` 时 owner 字典序（与效果 / 行为端口同一条装配纪律）。
#[test]
fn contributors_are_ordered_by_prio_then_owner() {
    let owners: Vec<&str> = contributors().iter().map(|c| c.owner()).collect();
    assert_eq!(owners, ["tests/alpha", "tests/omega"]);
}

/// 草稿**不带信封**：轮号与时刻只在宿主落格那一刻成立（D-252）。
///
/// 这里按宿主的落格写法走一遍——`into_event` 只交主体与形状，`turn` / `ts` 由写方盖。
#[test]
fn draft_carries_no_envelope_and_the_host_stamps_it() {
    let at = at();
    let contributed = Contribution::new(draft("d-1"));
    assert_eq!(contributed.actor, None);
    let mut event = contributed.fact.into_event(at.principal);
    event.turn = 3;
    event.ts = at.now_ms;
    assert_eq!(event.actor, "session/a");
    assert_eq!(event.event_id, "d-1");
    assert_eq!(event.kind, "test.contributed");
    assert_eq!(event.produced_by, Some(7));
    assert_eq!(event.turn, 3);
    assert_eq!(event.ts, 1_700);
}

/// 能力可以声明归属人（记忆的事实归人，不归跑轮主体）。
#[test]
fn actor_can_be_declared_by_the_capability() {
    let at = at();
    let contributed = Contribution::new(draft("d-2")).with_actor("person/local");
    let mut event = contributed
        .fact
        .into_event(contributed.actor.unwrap_or(at.principal));
    event.turn = 3;
    event.ts = at.now_ms;
    assert_eq!(event.actor, "person/local");
    assert_eq!(event.turn, 3);
    assert_eq!(event.ts, 1_700);
}

/// 缺省贡献是空：不关心收束的能力不必实现 `contribute`。
#[test]
fn default_contribution_is_empty() {
    struct Silent;
    impl Contributor for Silent {
        fn owner(&self) -> &'static str {
            "tests/silent"
        }
    }
    assert!(Silent.contribute(&at()).is_empty());
}

/// 贡献者拿到的是纯值，而且**只有这一刻的值**：没有 store、没有句柄、没有轮号、
/// 没有配置包（D-252 的两条否定在此钉住）。
#[test]
fn contribute_takes_pure_values() {
    let at = at();
    assert_eq!(at.face, SettleFace::Generated);
    assert_eq!(at.anchor, "t3");
    assert_eq!(at.user_seq, 42);
    assert_eq!(at.response, Some("答了什么"));
    let all = contributors();
    let early = all.iter().find(|c| c.owner() == "tests/alpha").unwrap();
    let drafts = early.contribute(&at);
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].fact.key, "t3-a");
}
