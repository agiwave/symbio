//! `cost.ledger` 的单测：**数字**在这里，契约在读方（`loop` 熔断 / `session` 成本列）。
//!
//! 与实现**同级**分文件（`decl.rs` + 同级 `decl.test.rs`）。本文件钉的是「算出来是多少」：
//! 按主体累计、笔数同账、as-of 之后不算、`cost_ms == 0` 不进账、未记账的主体在书里
//! 根本没有那一行、同一切片永远同一本账（N1）。
//!
//! ## 为什么手工给 `ts`
//!
//! 真实链路里 `ts` 由入库时刻填、`seq` 由存储的唯一写入口分配。本 crate 不链接存储（也不许
//! 链接：`E-009`），所以 as-of 用的 `ts` 在构造后手工补——测的是**折叠**，不是入库。折叠不读
//! `seq`，切片顺序就是它看到的顺序。

use super::*;
use symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL;
use symbio_plugin_api::{Entity, Verb};

/// 一条带实测耗时的事件。`kind` / `entity` / `verb` 都随便填：折叠只认信封上的
/// `actor` / `cost_ms` / `ts`，**不认识事件的名字**——这正是它读不了任何载荷的原因，
/// 也是 `reads: &[]` 合法的原因。
fn billed(id: &str, principal: &str, cost_ms: u64, ts: i64) -> Event {
    Event::pending(
        id,
        EVENT_CHAT_ASSISTANT_FINAL,
        Entity::Scope,
        Verb::Closed,
        principal,
    )
    .with_ts(ts)
    .with_cost_ms(cost_ms)
}

fn row<'a>(v: &'a Value, principal: &str) -> &'a Value {
    &v["by_principal"][principal]
}

/// 按主体累计：两个主体各记各的，全局行是两者的和。
///
/// 数字与搬出前 `Projection<CostLedgerView>` 的同名用例逐字相同（`session/a` 500ms 两笔、
/// `total_ms` 550）——迁移改的是**住在哪个 crate**，不是算式。
#[test]
fn ledger_accumulates_cost_per_principal() {
    let events = vec![
        billed("e0", "session/a", 300, 10),
        billed("e1", "session/a", 200, 20),
        billed("e2", "session/b", 50, 30),
    ];
    let v = ledger(&events, i64::MAX);
    assert_eq!(row(&v, "session/a")["spent_ms"], json!(500));
    assert_eq!(row(&v, "session/a")["entries"], json!(2));
    assert_eq!(row(&v, "session/b")["spent_ms"], json!(50));
    assert_eq!(v["total_ms"], json!(550));
}

/// 未记账的主体在书里**没有那一行**：折叠不负责把它折成零账。
///
/// 「这一格没记过」是读方的口径（闸门读成 0，出口读成空表），不是本视图的缺席态——
/// 缺席态只有四个，都在读口那一侧回答（`D-284`）。
#[test]
fn an_unbilled_principal_has_no_row() {
    let v = ledger(&[billed("e0", "session/a", 300, 10)], i64::MAX);
    assert_eq!(
        v["by_principal"].get("session/ghost"),
        None,
        "零账不该被折成一行——那会让空书和满书长得一样"
    );
    assert_eq!(v["total_ms"], json!(300));
}

/// as-of 之后的事件不进账：与其余读数同口径，「截至何时」由调用方给。
#[test]
fn ledger_respects_as_of() {
    let events = vec![
        billed("e0", "session/a", 300, 10),
        billed("e1", "session/a", 700, 900),
    ];
    let v = ledger(&events, 500);
    assert_eq!(row(&v, "session/a")["spent_ms"], json!(300));
    assert_eq!(
        row(&v, "session/a")["entries"],
        json!(1),
        "as-of 之外不算一笔"
    );
    assert_eq!(v["total_ms"], json!(300));
}

/// `cost_ms == 0` 不进账：桩与零延迟路径的诚实值就是 0，记成一笔会读出
/// 「花过钱而金额为零」的假账。
#[test]
fn a_zero_cost_event_is_not_a_row() {
    let v = ledger(
        &[
            billed("e0", "session/a", 0, 10),
            billed("e1", "session/a", 5, 20),
        ],
        i64::MAX,
    );
    assert_eq!(row(&v, "session/a")["entries"], json!(1));
    assert_eq!(row(&v, "session/a")["spent_ms"], json!(5));
}

/// 空切片 ⇒ 空书，不是缺席、不是错误。
#[test]
fn an_empty_slice_is_an_empty_ledger() {
    assert_eq!(
        ledger(&[], i64::MAX),
        json!({ "by_principal": {}, "total_ms": 0 })
    );
}

/// N1 双跑一致：纯函数 ⇒ 同一切片永远折出同一本账（字节也一样）。
#[test]
fn the_same_slice_always_gives_the_same_ledger() {
    let events = vec![
        billed("e1", "session/b", 50, 30),
        billed("e0", "session/a", 300, 10),
        billed("e2", "session/a", 200, 20),
    ];
    assert_eq!(ledger(&events, i64::MAX), ledger(&events, i64::MAX));
}

/// 声明形状：只读信封（`reads` 空表）、不替事实关门（`evidence` 空）、没有实例级读者
/// （`agg` 缺席，见 `decl.rs` 的「为什么没有 `agg` 那一半」），切点取全仓同一个值。
#[test]
fn the_view_is_declared_as_an_envelope_only_fold_with_no_merge_half() {
    assert_eq!(VIEWS.len(), 1);
    let v = &VIEWS[0];
    assert_eq!(v.name, COST_LEDGER);
    assert!(v.reads.is_empty(), "台账只读信封，一个载荷字段都不碰");
    assert!(v.evidence.is_empty(), "读数不替任何事实关门");
    assert!(v.agg.is_none(), "成本列没有实例级读者");
    assert_eq!(v.cost, COST, "切点全仓只有一个值");
}
