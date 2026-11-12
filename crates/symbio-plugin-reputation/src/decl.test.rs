//! `reputation.ledger` 的单测：**数字**在这里，契约在读方（`loop` 淘汰回路 / `session` 声誉列）。
//!
//! 与实现**同级**分文件（`decl.rs` + 同级 `decl.test.rs`）。本文件钉的是「算出来是多少」：
//! 账记在立约方头上、收束经 `id` 配对、认不出的了结不进账、自报（`asserted`）永远不进账、
//! as-of 之后不算、未立约的主体在书里根本没有那一行、同一切片永远同一本账（N1）。
//!
//! 数字与搬出前 `Projection<ReputationView>` 的同名用例逐字相同（`sdk` 那三条用例随折叠
//! 一起搬到这里）——迁移改的是**住在哪个 crate**，不是算式。
//!
//! ## 为什么手工给 `ts`
//!
//! 真实链路里 `ts` 由入库时刻填、`seq` 由存储的唯一写入口分配。本 crate 不链接存储（也不许
//! 链接：`E-009`），所以 as-of 用的 `ts` 在构造后手工补——测的是**折叠**，不是入库。折叠不读
//! `seq`，切片顺序就是它看到的顺序。
//!
//! ## 为什么本文件要交一份格位夹具
//!
//! 插件自己的测试二进制只链接本 crate：`commitment.*` 的格位由 `loop` 交（它是那一族的
//! owner，也是会话统计的读方）。不在这里 ⇒ `seal` 对本视图的 `reads` 报 `UnknownFact`，
//! 整张表失败（装载期检查），连「名字在装配表里」这条也证不了。夹具补的正是完整进程里
//! 由 `loop` 提供的那一半，只在 `cfg(test)` 生效。

use super::{
    ledger, COST, EVENT_COMMITMENT_BROKEN, EVENT_COMMITMENT_OPENED, EVENT_COMMITMENT_RELEASED,
    OWNER, REPUTATION_LEDGER, VIEWS,
};
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, LatencyTier, PluginDeclarations, Verb, EVENT_FAILED, EVENT_REQUEST,
    EVENT_RESPONSE, EVENT_SENT,
};
use symbio_testkit::EVENT_RETRY;

/// `loop` 拥有的三条承诺格（本二进制不链接它 ⇒ 夹具补格位）。
///
/// 格位与 `crates/symbio-plugin-loop/src/control_decl.rs` 的 `EVENTS` 逐条同构；
/// `commitment.asserted` 也一并交上——它**不是**本视图的输入，夹具要交的是「这一族在
/// 完整进程里长什么样」，漏一格就等于替折叠免了一次「名字对不对」的检查。
const HARNESS_EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_COMMITMENT_OPENED, (Entity::Commitment, Verb::Opened)),
    EventDecl::new(
        EVENT_COMMITMENT_RELEASED,
        (Entity::Commitment, Verb::Closed),
    ),
    EventDecl::new(EVENT_COMMITMENT_BROKEN, (Entity::Commitment, Verb::Closed)),
    EventDecl::new("commitment.asserted", (Entity::Commitment, Verb::Asserted)),
];

/// 内核协议五格的归宿行夹具（与 `symbio-plugin-skill` / `notes` 的同名夹具同形）。
///
/// 内核总是声明 `request` / `sent` / `response` / `failed` / `retry` 五条事实，它们的
/// 消费者行住在 `loop`；本二进制不链接它 ⇒ 缺行会报 `OrphanFact`，整张表失败
/// （装载期检查），连「本视图在装配表里」这一条也证不了。
fn harness_protocol_rows() -> &'static [Decl] {
    static ROWS: std::sync::OnceLock<Vec<Decl>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| {
        [
            EVENT_REQUEST,
            EVENT_SENT,
            EVENT_RESPONSE,
            EVENT_FAILED,
            EVENT_RETRY,
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

fn harness_declarations() -> PluginDeclarations<'static> {
    symbio_plugin_api::PluginDeclarations {
        events: HARNESS_EVENTS,
        facts: &[],
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: harness_protocol_rows(),
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-reputation-test-harness",
    harness_declarations
);

/// 立约：写方是 `CommitmentKeeper::offer`，载荷 `{id, from, to, promise}`、actor = `from`。
fn offered(id: &str, from: &str, ts: i64) -> Event {
    Event::pending(
        format!("c-offer-{id}"),
        EVENT_COMMITMENT_OPENED,
        Entity::Commitment,
        Verb::Opened,
        from,
    )
    .with_ts(ts)
    .with_payload(serde_json::json!({
        "id": id, "from": from, "to": "user:local", "promise": "把这段复查看完",
    }))
}

/// 守约收束（载荷只有 `id`——`from` 是立约时的事实，收束时不重抄）。
fn kept(id: &str, from: &str, ts: i64) -> Event {
    Event::pending(
        format!("c-close-{id}"),
        EVENT_COMMITMENT_RELEASED,
        Entity::Commitment,
        Verb::Closed,
        from,
    )
    .with_ts(ts)
    .with_payload(serde_json::json!({ "id": id }))
}

/// 违约收束（必须带 `why`：违约可被观测是这套机制的全部前提，DE-038 §5）。
fn breached(id: &str, from: &str, why: &str, ts: i64) -> Event {
    Event::pending(
        format!("c-close-{id}"),
        EVENT_COMMITMENT_BROKEN,
        Entity::Commitment,
        Verb::Closed,
        from,
    )
    .with_ts(ts)
    .with_payload(serde_json::json!({ "id": id, "why": why }))
}

/// 对等宣告（`asserted` 格）：主体自己说「这条约我守了」。
fn asserted(id: &str, from: &str, ts: i64) -> Event {
    Event::pending(
        format!("c-assert-{id}"),
        "commitment.asserted",
        Entity::Commitment,
        Verb::Asserted,
        from,
    )
    .with_ts(ts)
    .with_payload(serde_json::json!({
        "id": id, "from": from, "statement": "已告知：守约",
    }))
}

fn row<'a>(v: &'a serde_json::Value, principal: &str) -> &'a serde_json::Value {
    &v["by_principal"][principal]
}

/// 账记在**立约方**头上：委托由父主体发起，违约算在许诺的那一个身上，而不是事件写出方。
///
/// 数字与搬出前的同名用例一致：`session/main` 立两条、守一条、违约一条 ⇒ 分 0。
#[test]
fn the_ledger_books_the_promise_maker() {
    let events = vec![
        offered("c1", "session/main", 10),
        kept("c1", "session/main", 11),
        offered("c2", "session/main", 12),
        breached("c2", "session/main", "子智能体没起来", 13),
        offered("c3", "agent:peer-b", 14),
    ];
    let v = ledger(&events, i64::MAX);
    assert_eq!(
        row(&v, "session/main"),
        &serde_json::json!({
            "principal": "session/main",
            "offered": 2,
            "kept": 1,
            "broken": 1,
            "score": 0,
        }),
        "两条各了结一次：守约 − 违约 = 0"
    );
    assert_eq!(
        row(&v, "agent:peer-b"),
        &serde_json::json!({
            "principal": "agent:peer-b",
            "offered": 1,
            "kept": 0,
            "broken": 0,
            "score": 0,
        }),
        "在途的那条不进 `kept` / `broken`，但 `offered` 是事实"
    );
}

/// 自报不进账：`asserted` 与两条收束格同在 `commitment` 实体上，它却**不是**一次了结。
///
/// 这是本视图与 DE-038 §2「声誉 = 可被别人观测的那一份」的兑现处：主体能宣告自己守约，
/// 宣告改变不了账。少这一条判据，淘汰回路读到的就是被审判者自己填的分数。
#[test]
fn a_self_report_never_moves_the_ledger() {
    let events = vec![
        offered("c1", "agent:peer-b", 10),
        asserted("c1", "agent:peer-b", 11),
        asserted("c2", "agent:peer-b", 12),
        // 换个格子、换个实体都没用：本视图的输入闭集是那三条名字。
        Event::pending(
            "self1",
            "reputation.reported",
            Entity::Verdict,
            Verb::Asserted,
            "agent:peer-b",
        )
        .with_ts(13)
        .with_payload(serde_json::json!({ "principal": "agent:peer-b", "score": 99 })),
    ];
    let v = ledger(&events, i64::MAX);
    assert_eq!(row(&v, "agent:peer-b")["kept"], serde_json::json!(0));
    assert_eq!(row(&v, "agent:peer-b")["broken"], serde_json::json!(0));
    assert_eq!(
        row(&v, "agent:peer-b")["offered"],
        serde_json::json!(1),
        "只有那一条真立过的约计数"
    );
    assert_eq!(
        v["by_principal"].get("reputation.reported"),
        None,
        "自报的 99 分既进不了数、也造不出一行（DE-043 §6 断言 2/4）"
    );
}

/// 认不出的了结 ⇒ **跳过**，不猜「大概履行了吧」：切片被截断或写方漏了 `opened` 时，
/// 猜一次就往 `kept` 里掺一次假账（与 `commitment_records` 同一条 fail-closed）。
#[test]
fn a_closure_without_its_offer_is_skipped() {
    let v = ledger(
        &[
            kept("ghost-1", "session/main", 10),
            breached("ghost-2", "session/main", "没有对应的立约", 11),
        ],
        i64::MAX,
    );
    assert_eq!(
        v["by_principal"].get("session/main"),
        None,
        "没立过约的主体不该被折成一行——那会让假了结凭空造出一本书"
    );
    assert_eq!(v["by_principal"], serde_json::json!({}));
}

/// 未立约的主体在书里**没有那一行**：折叠不负责把它折成零分。
///
/// 「这一格没立过约」是读方的口径（淘汰回路读成「不淘汰」，出口读成零分条目），
/// 不是本视图的缺席态——缺席态只有四个，都在读口那一侧回答（`D-284`）。
#[test]
fn a_principal_that_never_promised_has_no_row() {
    let v = ledger(&[offered("c1", "session/main", 10)], i64::MAX);
    assert_eq!(
        v["by_principal"].get("session/ghost"),
        None,
        "零分不该被折成一行——那会让空书和满书长得一样"
    );
}

/// as-of 之后的立约与收束都不进账：未来的了结不影响当前的声誉（与 `recall` 同口径）。
#[test]
fn the_ledger_respects_as_of() {
    let events = vec![
        offered("c1", "session/main", 10),
        kept("c1", "session/main", 11),
        offered("c2", "session/main", 900),
        breached("c2", "session/main", "未来的一次违约", 901),
    ];
    let v = ledger(&events, 500);
    assert_eq!(
        row(&v, "session/main"),
        &serde_json::json!({
            "principal": "session/main",
            "offered": 1,
            "kept": 1,
            "broken": 0,
            "score": 1,
        }),
        "as-of 之外那一幕还没发生"
    );
}

/// N1 双跑一致：纯函数 ⇒ 同一切片永远折出同一本账（字节也一样）。
#[test]
fn the_same_slice_always_gives_the_same_ledger() {
    let events = vec![
        breached("c2", "session/main", "子智能体没起来", 30),
        offered("c2", "session/main", 20),
        offered("c1", "session/main", 10),
        kept("c1", "session/main", 11),
    ];
    assert_eq!(ledger(&events, i64::MAX), ledger(&events, i64::MAX));
}

/// 键序由 `BTreeMap` 给（主体字典序），空切片 ⇒ `{ by_principal: {} }`：
/// 「这条事实源里还没人立约」是**有据的空**，与「读不出来」是两件事。
#[test]
fn rows_come_out_in_key_order_and_an_empty_slice_is_an_empty_book() {
    let events = vec![
        offered("c1", "session/main", 10),
        offered("c2", "agent:peer-b", 11),
        offered("c3", "session/aaa", 12),
    ];
    let v = ledger(&events, i64::MAX);
    let keys: Vec<&str> = v["by_principal"]
        .as_object()
        .expect("by_principal 是对象")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec!["agent:peer-b", "session/aaa", "session/main"],
        "字节确定：同一份事实源永远同一串键序"
    );
    assert_eq!(
        ledger(&[], i64::MAX),
        serde_json::json!({ "by_principal": {} })
    );
}

/// 声明形状：读的就是那四条 `reads` 对（断言格读法，见 `decl.rs` 的 `D-282` 一节）、
/// 台账不替事实关门（`evidence` 空）、没有实例级读者（`agg` 缺席）、切点取全仓同一个值。
#[test]
fn the_view_is_declared_over_the_three_commitment_cells() {
    assert_eq!(VIEWS.len(), 1);
    let v = &VIEWS[0];
    assert_eq!(v.name, REPUTATION_LEDGER);
    assert_eq!(
        v.reads,
        &[
            (EVENT_COMMITMENT_OPENED, "id"),
            (EVENT_COMMITMENT_OPENED, "from"),
            (EVENT_COMMITMENT_RELEASED, "id"),
            (EVENT_COMMITMENT_BROKEN, "id"),
        ],
        "配对靠 `id`、记账靠 `from`：这四对就是折叠的全部输入"
    );
    assert!(v.evidence.is_empty(), "读数不替任何事实关门");
    assert!(v.agg.is_none(), "声誉列没有实例级读者");
    assert_eq!(v.cost, COST, "切点全仓只有一个值");
}

/// 台账对外可读：名字在冻结表里，owner 是本插件。
///
/// 读方是 `loop` 的淘汰回路与 `session` 的声誉列（跨插件读口按名字取）；数字判据在
/// 上面的 owner 用例，「生产二进制里读得到真数」由 e2e `t30` / `t42` / `t71` / `t74` /
/// `t75` 钉。
#[test]
fn the_ledger_is_sealed_under_this_owner() {
    let frozen = symbio_testkit::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert!(frozen.view(REPUTATION_LEDGER).is_some());
    assert_eq!(frozen.view_owner(REPUTATION_LEDGER), Some(OWNER));
    // 读的三条承诺格在本二进制里是**断言格读法**（有 `EventDecl`、无 `FactDecl`）：
    // 拼错名字仍红（`Gap::UnknownFact`），没有形状可校字段则进这份名单（`D-282`）。
    let reads = frozen.assertion_reads();
    for fact in [
        EVENT_COMMITMENT_OPENED,
        EVENT_COMMITMENT_RELEASED,
        EVENT_COMMITMENT_BROKEN,
    ] {
        assert!(
            reads.iter().any(|r| r.fact == fact),
            "`{fact}` 应被记成断言格读法而不是静默跳过：{reads:?}"
        );
    }
}
