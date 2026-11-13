//! 能力包声明的判据：本插件交的名字在装配表里按自己的格位解析，行与视图都接得上。
//!
//! ## 为什么本文件要交一份「宿主行」夹具
//!
//! 插件自己的测试二进制只链接本 crate：`memory.encoded` 的**事实形状**自 DE-026 D-263
//! 起住情景记忆能力包（`crates/symbio-plugin-memory`）。不在这里 ⇒ `seal` 报
//! `OrphanFact` / `UnknownSource` / `UnknownFact` / `FactWithoutEvent`，
//! 整张表失败（装载期检查）。夹具补的正是完整进程里由那个 owner 提供的那一半
//! ——事实形状与归宿行。它与 notes 的同名夹具同形，只在 `cfg(test)` 生效。
//!
//! **不补 `plugin_row` 执行体与 `user.message`**：本包的 `skill.recall` 行随 B2
//! （`60cedd50`）与它的实现 `row.rs`、`turn.recalled` 绑定、`skill.answered` 产出
//! 一起退场，本包今天不再有 `by = plugin_row` 的行。夹具不声明生产上不存在的东西。

use super::{
    calibration_ledger, calibration_ledger_view, confidence_of, retreating_skills, SkillStat,
    EVENTS, EVENT_MEMORY_CALIBRATED, FACTS, OWNER, ROWS, SKILL_CALIBRATION, SKILL_COMPILED,
    SKILL_COMPILE_SWITCH, SKILL_RETREAT, SKILL_TAG, VIEWS,
};
use symbio_plugin_api::events::EVENT_MEMORY_ENCODED;
use symbio_plugin_api::events::EVENT_SESSION_CONFIG;
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, Event, EventDecl, FactDecl, PluginDeclarations, Verb, ViewDecl};

/// loop 拥有的事实格（本二进制不链接它 ⇒ 夹具补格位与载荷字段）。
const HARNESS_EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_MEMORY_ENCODED, (Entity::Memory, Verb::Opened)),
    // `session.config` 的格位夹具：完整进程由会话插件交（`D-389` 移交时格位 /
    // 载荷原样照抄），本二进制不链接它 ⇒ `skill.compile_switch` 读它会报
    // `UnknownFact`。它只有事件格位、没有事实声明 ⇒ 视图 reads 冻结为
    // assertion read（`D-282`），字段形状无从校验也不必校验。
    EventDecl::new(EVENT_SESSION_CONFIG, (Entity::Scope, Verb::Asserted)),
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
];

const HARNESS_VIEWS: &[ViewDecl] = &[];

fn harness_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: HARNESS_EVENTS,
        facts: HARNESS_FACTS,
        views: HARNESS_VIEWS,
        ports: &[],
        // 本包没有 `by = plugin_row` 的行（见模块头的退场说明）⇒ 也不声明该执行体：
        // 声明一个没人绑的 actor 只会让 seal 报 `OrphanActor`。
        actors: Vec::new(),
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
        reads: &[],
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
            symbio_testkit::EVENT_RETRY,
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
                reads: &[],
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
}

/// 校准观测的格位自本能力迁入起由本插件交（loop 不再声明它）。
#[test]
fn the_calibration_event_is_owned_here() {
    let frozen = symbio_testkit::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(EVENT_MEMORY_CALIBRATED),
        Some((Entity::Memory, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(EVENT_MEMORY_CALIBRATED), Some(OWNER));
}

/// 回退视图在冻结表里，owner 是本插件（是否公开由 `PLUGIN.yml` 决定，不在这条判据里）。
#[test]
fn the_retreat_view_is_sealed_under_this_owner() {
    let frozen = symbio_testkit::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert!(frozen.view(SKILL_RETREAT).is_some());
    assert_eq!(frozen.view_owner(SKILL_RETREAT), Some(OWNER));
}

/// 编译两视图（已编译数 / 开关）同样在冻结表里，owner 是本插件（`D-489`）。
#[test]
fn the_compile_views_are_sealed_under_this_owner() {
    let frozen = symbio_testkit::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert!(frozen.view(SKILL_COMPILED).is_some());
    assert_eq!(frozen.view_owner(SKILL_COMPILED), Some(OWNER));
    assert!(frozen.view(SKILL_COMPILE_SWITCH).is_some());
    assert_eq!(frozen.view_owner(SKILL_COMPILE_SWITCH), Some(OWNER));
}

/// 已编译数只数 `tag:"skill"` 的编码事实——判据随口径一起回 owner（`D-489`）。
///
/// 从 session 迁来时逐字保持原口径：按 `tag` 认，不按整格数；空事实源 ⇒ 有据的零。
#[test]
fn compiled_count_only_counts_skill_tagged_facts() {
    let encoded = |id: &str, tag: &str| {
        Event::pending(
            id.to_string(),
            EVENT_MEMORY_ENCODED,
            Entity::Memory,
            Verb::Opened,
            "session/main",
        )
        .with_payload(serde_json::json!({ "tag": tag, "content": id }))
    };
    let events = [
        encoded("sk-1", SKILL_TAG),
        encoded("sk-2", SKILL_TAG),
        encoded("mem-1", "fact"),
    ];
    assert_eq!(
        super::compiled_count(&events, i64::MAX),
        serde_json::json!(2),
        "只有 tag=skill 的两条算技能"
    );
    assert_eq!(
        super::compiled_count(&[], i64::MAX),
        serde_json::json!(0),
        "空事实源 ⇒ 有据的零"
    );
}

/// 开关读法只有一份（`contributor::compile_enabled`）：最后一条 `session.config`
/// 的 `skill_compile_enabled`，缺省 `false`（出厂关，`D-427`）。
#[test]
fn compile_switch_reads_the_latest_config_event() {
    let cfg = |id: &str, on: bool| {
        Event::pending(
            id.to_string(),
            EVENT_SESSION_CONFIG,
            Entity::Scope,
            Verb::Asserted,
            "session/main",
        )
        .with_payload(serde_json::json!({ "skill_compile_enabled": on }))
    };
    assert_eq!(
        super::compile_switch(&[cfg("c-1", true), cfg("c-2", false)], i64::MAX),
        serde_json::json!(false),
        "最后一条为准"
    );
    assert_eq!(
        super::compile_switch(&[cfg("c-1", false), cfg("c-2", true)], i64::MAX),
        serde_json::json!(true)
    );
    assert_eq!(
        super::compile_switch(&[], i64::MAX),
        serde_json::json!(false),
        "没有配置事件 ⇒ 出厂关"
    );
}

/// 引用关系不虚：夹具里那条宿主格不是本能力的事实声明。
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
            "session/main",
        );
        e.seq = Some(symbio_testkit::Seq::new(seq));
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
            "session/main",
        );
        e.seq = Some(symbio_testkit::Seq::new(seq));
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

// ── 校准账的算式（数归 owner）──────────────────────────────────────────
//
// 这段判据钉的是折叠**最容易走样的三个地方**，它们都只在算式这一侧可判：
//
// ① 只认这条名字。按载荷字段认输入（「带 `skill_id` + `fallback` 就算一条观测」）把
//    正确性押在一个巧合上——别的格子恰好不带这两个字段。巧合被打破后**没有任何东西会红**：
//    一个新格子带同名字段，置信度就凭空被算歪。
// ② 名字对了但字段缺 ⇒ 不参与统计，而不是记成「未回退」——后者让 `uses` 涨而
//    `fallbacks` 不涨，凭空造出高置信度，正是回退机制失效的形态。
// ③ 回退率、置信度与 as-of 各钉一个已知期望值。

/// 一条 `memory × asserted` 的校准观测（这一族格子是共享的，见判据 ①）。
fn observed_event(id: &str, kind: &str, payload: serde_json::Value, ts: i64) -> Event {
    Event::pending(
        id.to_string(),
        kind,
        Entity::Memory,
        Verb::Asserted,
        "session/main",
    )
    .with_ts(ts)
    .with_payload(payload)
}

/// 一条正常形状的技能校准观测。
fn observed(seq: i64, skill_id: &str, fallback: bool) -> Event {
    observed_event(
        &format!("c-{seq}"),
        EVENT_MEMORY_CALIBRATED,
        serde_json::json!({ "skill_id": skill_id, "fallback": fallback }),
        seq,
    )
}

#[test]
fn only_the_calibration_cell_feeds_the_ledger() {
    let events = [
        observed(1, "sk-1", false),
        // 两个「长得像」的别的格子：一个带全两个字段，一个只带 `skill_id`。
        // 名字按线上拼法造，不从别的插件借常量（跨插件零依赖）。
        observed_event(
            "r-2",
            "memory.recalled",
            serde_json::json!({ "skill_id": "sk-2", "fallback": true }),
            2,
        ),
        observed_event(
            "e-3",
            "memory.consolidated",
            serde_json::json!({ "skill_id": "sk-3" }),
            3,
        ),
    ];
    let ledger = calibration_ledger(&events, i64::MAX);
    assert_eq!(
        ledger.keys().cloned().collect::<Vec<_>>(),
        vec!["sk-1".to_string()],
        "只有 `memory.calibrated` 进统计；带同名字段的 `memory.recalled` 不算"
    );
    assert_eq!(
        ledger["sk-1"],
        SkillStat {
            uses: 1,
            fallbacks: 0
        }
    );
}

#[test]
fn an_observation_missing_a_field_is_not_counted_as_no_fallback() {
    let events = [
        observed(1, "sk-1", false),
        observed_event(
            "c-2",
            EVENT_MEMORY_CALIBRATED,
            serde_json::json!({ "skill_id": "sk-1" }),
            2,
        ),
    ];
    let ledger = calibration_ledger(&events, i64::MAX);
    assert_eq!(ledger["sk-1"].uses, 1, "缺字段的事件不得让 uses 增长");
    assert_eq!(ledger["sk-1"].fallbacks, 0);
}

#[test]
fn confidence_follows_the_known_actions() {
    let events = [
        observed(1, "sk-1", false),
        observed(2, "sk-1", true),
        observed(3, "sk-1", true),
    ];
    assert!(
        (confidence_of(&events, i64::MAX, "sk-1") - (1.0 - 2.0 / 3.0)).abs() < 1e-12,
        "三次里回退两次 ⇒ 置信度 1/3"
    );
    // 账里没有这条技能 ⇒ 零使用 ⇒ 1.0（不给新技能判死刑）。
    assert_eq!(confidence_of(&events, i64::MAX, "never-used"), 1.0);
}

#[test]
fn the_ledger_honours_as_of() {
    let events = [observed(5, "sk-1", true)];
    assert!(
        calibration_ledger(&events, 4).is_empty(),
        "as-of 之前的观测不得计入"
    );
    assert_eq!(calibration_ledger(&events, 5)["sk-1"].fallbacks, 1);
}

/// 视图交出的形状（会话统计的校准列按它排版）：键字典序、每条带 `skill_id`，
/// 没判过任何技能 ⇒ `{ by_skill: {} }` 而不是缺列。
#[test]
fn the_calibration_view_emits_the_ledger_in_key_order() {
    let events = [
        observed(1, "sk-b", false),
        observed(2, "sk-a", true),
        observed(3, "sk-a", false),
    ];
    assert_eq!(
        calibration_ledger_view(&events, i64::MAX),
        serde_json::json!({
            "by_skill": {
                "sk-a": { "skill_id": "sk-a", "uses": 2, "fallbacks": 1 },
                "sk-b": { "skill_id": "sk-b", "uses": 1, "fallbacks": 0 },
            }
        }),
    );
    assert_eq!(
        calibration_ledger_view(&[], i64::MAX),
        serde_json::json!({ "by_skill": {} }),
        "没编过技能的会话读成有据的空"
    );
}

/// 账对外可读：名字在冻结表里，owner 是本插件。
///
/// 读方是 `crates/symbio-plugin-session` 的校准列（跨插件读口按名字取）；
/// 数字判据在上面的 owner 用例，「名字在装配表里」由这条钉，
/// 「生产二进制里读得到真数」由 e2e `t35` 钉。
#[test]
fn the_calibration_view_is_sealed_under_this_owner() {
    let frozen = symbio_testkit::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert!(frozen.view(SKILL_CALIBRATION).is_some());
    assert_eq!(frozen.view_owner(SKILL_CALIBRATION), Some(OWNER));
}
