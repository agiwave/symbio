//! 能力包声明的判据：本插件交的名字在装配表里按自己的格位解析，行与视图都接得上。
//!
//! ## 为什么本文件要交一份「宿主行」夹具
//!
//! 插件自己的测试二进制只链接本 crate：`turn.recalled` 视图与 `turn.stage` /
//! `user.message` 事实的归宿行住在 `crates/symbio-plugin-loop`，`memory.encoded` 的
//! 格位也在那里；它的**事实形状**自 DE-026 D-263 起住情景记忆能力包
//! （`crates/symbio-plugin-memory_episodic`）。不在这里 ⇒ `seal` 报 `OrphanFact` /
//! `UnknownSource` / `UnknownFact` / `FactWithoutEvent`，整张表失败（DE-015 §3.4）。
//! 夹具补的正是完整进程里由那两个 owner 提供的那一半——视图、事实形状与通用行执行体
//! 的声明。它与 notes 的同名夹具同形，只在 `cfg(test)` 生效。

use super::{
    retreating_skills, EVENTS, FACTS, OWNER, ROWS, SKILL_ANSWERED, SKILL_RECALL, SKILL_RETREAT,
    SKILL_TAG, VIEWS,
};
use symbio_plugin_api::events::{EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::ring::{Decl, Lit, Pattern};
use symbio_plugin_api::{
    ActorDecl, Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// loop 拥有的两条事实格（本二进制不链接它 ⇒ 夹具补格位与载荷字段）。
const HARNESS_EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_MEMORY_ENCODED, (Entity::Memory, Verb::Opened)),
    EventDecl::new("turn.stage", (Entity::Turn, Verb::Asserted)),
    EventDecl::new("user.message", (Entity::Turn, Verb::Opened)),
];

const HARNESS_FACTS: &[FactDecl] = &[
    // `memory.encoded` 的**形状**夹具：完整进程由情景记忆能力包申报（D-263），本
    // 二进制不链接它——`skill.retreat` 读它的 `tag` / `skill_id`，缺形状会报
    // `UnknownFact`。字段表与该包 `decl.rs` 的申报同构。
    FactDecl {
        name: EVENT_MEMORY_ENCODED,
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "content",
            "tag",
            "generation",
            "vis_scope",
            "skill_id",
            "trigger",
        ],
    },
    FactDecl {
        name: "turn.stage",
        port: None,
        key: &[],
        expand: &[],
        fields: &["stage", "round", "recalled", "resumed"],
    },
    FactDecl {
        name: "user.message",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "id"],
    },
];

/// 宿主公开的召回视图：完整进程里住 loop 的 `control_decl::VIEWS`。
///
/// 本能力只按名字读它的输出（`recall` / `tag` / `stage` / `resumed`），夹具的形状
/// 与那份定义同构即可——`seal` 只查「行引用的名字存在、读面字段已声明」。
fn turn_recalled(_events: &[Event], _now: i64) -> serde_json::Value {
    serde_json::Value::Array(Vec::new())
}

const HARNESS_VIEWS: &[ViewDecl] = &[ViewDecl {
    name: "turn.recalled",
    ver: 1,
    reads: &[
        ("user.message", "id"),
        ("turn.stage", "stage"),
        ("turn.stage", "round"),
        ("turn.stage", "recalled"),
    ],
    evidence: &["turn.stage"],
    cost: 4096,
    eval: turn_recalled,
}];

fn harness_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: HARNESS_EVENTS,
        facts: HARNESS_FACTS,
        views: HARNESS_VIEWS,
        ports: &[],
        // 通用行执行体的声明由宿主给：行实现自己声明写入（`SkillRecall::writes`），
        // 装配期并进这份名单。本二进制补一份够用的 `ActorDecl`，模式取最宽档。
        actors: vec![ActorDecl {
            name: symbio_plugin_api::rows::PLUGIN_ROW,
            mode: LatencyTier::Deep,
            reads: &[],
            writes: &[EVENT_MEMORY_CALIBRATED, SKILL_ANSWERED],
        }],
        rows: harness_rows(),
    }
}

/// `memory.encoded` 的归宿行夹具：完整进程由情景记忆能力包交（D-263），本二进制
/// 不链接它 ⇒ 缺行会报 `OrphanFact`。形状与那张行同构（有即闭合，eq 为空）。
fn harness_encoded_sink_row() -> Decl {
    let pattern: &'static [Pattern] = Box::leak(Box::new([Pattern {
        source: EVENT_MEMORY_ENCODED,
        key: &[],
        eq: &[],
    }]));
    Decl {
        name: EVENT_MEMORY_ENCODED,
        prio: 200,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: pattern[0],
        until: pattern,
        unless: &[],
    }
}

fn harness_rows() -> &'static [Decl] {
    static ALL: std::sync::OnceLock<&'static [Decl]> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut v: Vec<Decl> = harness_protocol_rows().to_vec();
        v.push(harness_encoded_sink_row());
        Box::leak(v.into_boxed_slice())
    })
}

/// 内核协议五格的归宿行：内核总是声明这五条事实，而本二进制不链接 loop ⇒ 它们的
/// 消费者不在表里，`seal` 会报 `OrphanFact`。夹具补五条自读行（与 plugin-api
/// 自己的收集通道用例同形），让「声明事实有归宿」那条判据不挡住本文件要验的声明面。
fn harness_protocol_rows() -> &'static [Decl] {
    static ROWS: std::sync::OnceLock<Vec<Decl>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| {
        [
            symbio_plugin_api::EVENT_REQUEST,
            symbio_plugin_api::EVENT_SENT,
            symbio_plugin_api::EVENT_RESPONSE,
            symbio_plugin_api::EVENT_FAILED,
            symbio_plugin_api::EVENT_RETRY,
        ]
        .into_iter()
        .map(|name| {
            let pattern: &'static [Pattern] = Box::leak(Box::new([Pattern {
                source: name,
                key: &[],
                eq: &[],
            }]));
            Decl {
                name: Box::leak(format!("sink.{name}").into_boxed_str()),
                prio: 200,
                by: None,
                tier: LatencyTier::Reflex,
                expand: None,
                owes: pattern[0],
                until: pattern,
                unless: &[],
            }
        })
        .collect::<Vec<_>>()
    })
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-skill-test-harness",
    harness_declarations
);

/// 本能力包自己的表先自证：事件 / 事实 / 视图 / 行四块都有内容。
///
/// `memory.encoded` **不在**本包的 `FACTS` / `ROWS` 里（D-263）：形状的 owner 是
/// 情景记忆能力包，本包只是 `tag:"skill"` 那半的一个写方——一处申报两个 owner，
/// 完整进程的 `seal` 会报 `DuplicateFact`。
#[test]
fn the_skill_package_submits_its_own_surfaces() {
    assert!(!EVENTS.is_empty());
    assert!(FACTS.iter().any(|f| f.name == EVENT_MEMORY_CALIBRATED));
    assert!(
        !FACTS.iter().any(|f| f.name == EVENT_MEMORY_ENCODED),
        "编码事实的形状随记忆家族走，本包不再申报（D-263）"
    );
    assert!(
        !ROWS.iter().any(|r| r.name == EVENT_MEMORY_ENCODED),
        "归宿行同样只有一张，随形状 owner 走"
    );
    assert!(VIEWS.iter().any(|v| v.name == SKILL_RETREAT));
    assert!(ROWS.iter().any(|r| r.name == SKILL_RECALL));
}

/// `skill.answered` 落在 `Domain × Asserted`，owner 是本插件。
#[test]
fn the_answered_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(SKILL_ANSWERED),
        Some((Entity::Domain, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(SKILL_ANSWERED), Some(OWNER));
}

/// 校准观测的格位自本能力迁入起由本插件交（loop 不再声明它）。
#[test]
fn the_calibration_event_is_owned_here() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(EVENT_MEMORY_CALIBRATED),
        Some((Entity::Memory, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(EVENT_MEMORY_CALIBRATED), Some(OWNER));
}

/// 回退视图对外可读：名字在冻结表里，owner 是本插件。
#[test]
fn the_retreat_view_is_sealed_under_this_owner() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert!(frozen.view(SKILL_RETREAT).is_some());
    assert_eq!(frozen.view_owner(SKILL_RETREAT), Some(OWNER));
}

/// 召回行只认宿主公开的 `turn.recalled` 里属于技能的那些条目。
#[test]
fn the_recall_row_binds_to_the_host_view() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    let row = frozen
        .rows()
        .iter()
        .find(|d| d.name == SKILL_RECALL)
        .expect("召回行应在冻结表里");
    assert_eq!(row.owes.source, "turn.recalled");
    assert_eq!(row.owes.eq.len(), 1);
    assert_eq!(row.owes.eq[0].0, "tag");
    assert!(matches!(row.owes.eq[0].1, Lit::Str(tag) if tag == SKILL_TAG));
    assert_eq!(row.by, Some(symbio_plugin_api::rows::PLUGIN_ROW));
    assert_eq!(row.until.len(), 1);
    assert_eq!(row.until[0].source, SKILL_ANSWERED);
    assert_eq!(row.until[0].key, &["recall", "stage"]);
    assert!(row.until[0].eq.is_empty());
}

/// 引用关系不虚：夹具里那两条宿主格不是本能力的事实声明。
#[test]
fn harness_facts_do_not_overlap_with_our_own() {
    for fact in FACTS {
        assert!(
            !HARNESS_FACTS.iter().any(|h| h.name == fact.name),
            "{} 重复声明",
            fact.name
        );
    }
}

/// 回退视图只挑低于阈值的技能：新技能（零使用）不被判死刑，回退过的才进集合。
#[test]
fn only_low_confidence_skills_retreat() {
    let skill = |seq: u64, skill_id: &str| {
        let mut e = Event::pending(
            format!("s-{seq}"),
            EVENT_MEMORY_ENCODED,
            Entity::Memory,
            Verb::Opened,
            0,
            "session/main",
        );
        e.seq = Some(symbio_plugin_api::Seq::new(seq));
        e.ts = 1;
        e.payload = serde_json::json!({ "tag": SKILL_TAG, "skill_id": skill_id, "content": "答" });
        e
    };
    let calibrated = |seq: u64, skill_id: &str, fallback: bool| {
        let mut e = Event::pending(
            format!("c-{seq}"),
            EVENT_MEMORY_CALIBRATED,
            Entity::Memory,
            Verb::Asserted,
            0,
            "session/main",
        );
        e.seq = Some(symbio_plugin_api::Seq::new(seq));
        e.ts = 1;
        e.payload = serde_json::json!({ "skill_id": skill_id, "fallback": fallback });
        e
    };

    let events = [
        skill(10, "sk-steady"),
        skill(11, "sk-fresh"),
        skill(12, "sk-broken"),
        calibrated(20, "sk-steady", false),
        calibrated(21, "sk-broken", true),
    ];
    let out = retreating_skills(&events, 2);
    assert_eq!(
        out,
        serde_json::json!([{ "recall": 12, "skill_id": "sk-broken" }]),
        "只有回退过的技能进回退集合"
    );
}
