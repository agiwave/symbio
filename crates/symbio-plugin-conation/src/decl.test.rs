//! `conation` 视图的单测。
//!
//! 与实现**同级**分文件（`decl.rs` + 同级 `decl.test.rs`，见 `CONTRIBUTING.md`）。本文件钉
//! 五件事：欲的**升格靠溯源认**（不是靠 goal 字符串）、**as-of** 口径、**无溯源的欲可读但
//! 永不升格**（I2 边界）、**只声明升格过的那条**（不误伤同批的别的欲），以及 `D-281` 的
//! 那一格——**单片与多片走同一条算式**。
//!
//! ## 为什么手工给 `seq`
//!
//! 真实链路里 `seq` 由存储的唯一写入口分配（`Event::pending` 交出来时是 `None`）。本 crate
//! 不链接存储（也不许链接：HS-001），所以用例在构造后手工补序号——测的是**折叠**，不是入库。

use super::*;
use symbio_testkit::Seq;

const INITIATOR: &str = "session/autonomous";

/// 一条已表达、已带序号的欲事件。
fn expressed(id: &str, goal: &str, seq: u64, ts: i64) -> Event {
    let mut e = Event::pending(
        id,
        EVENT_CONATION_EXPRESSED,
        Entity::Conation,
        Verb::Opened,
        INITIATOR,
    )
    .with_ts(ts)
    .with_payload(json!({ "goal": goal }));
    e.seq = Some(Seq::new(seq));
    e
}

/// 由某条欲长出一个任务（`produced_by` 指回欲 seq——`open_long_goal` 的形状）。
fn opened_task(id: &str, task_id: &str, goal: &str, source: u64, ts: i64) -> Event {
    let mut e = Event::pending(id, EVENT_TASK_OPENED, Entity::Task, Verb::Opened, INITIATOR)
        .with_ts(ts)
        .with_produced_by(source)
        .with_payload(json!({ "task_id": task_id, "goal": goal }));
    e.seq = Some(Seq::new(9_000 + source));
    e
}

fn declared_goals(v: &Value) -> Vec<String> {
    v["declared"]
        .as_array()
        .expect("declared 是数组")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn task_id_at(v: &Value, i: usize) -> Option<String> {
    v["intents"][i]["task_id"].as_str().map(str::to_string)
}

/// 欲先入格、任务后长出来：升格前不算声明，升格后才算。
#[test]
fn an_expressed_intent_is_pending_until_a_task_derives_from_it() {
    let want = expressed("want-0", "把周报写了", 1, 100);
    let before = intents(std::slice::from_ref(&want), i64::MAX);
    assert_eq!(before["intents"].as_array().unwrap().len(), 1);
    assert_eq!(before["intents"][0]["goal"], "把周报写了");
    assert_eq!(task_id_at(&before, 0), None, "未升格 ⇒ 没有派生任务");
    assert!(
        declared_goals(&before).is_empty(),
        "光有欲不算声明——心跳照旧该开这一格"
    );

    let after = intents(
        &[want, opened_task("o-1", "hb-1", "把周报写了", 1, 200)],
        i64::MAX,
    );
    assert_eq!(task_id_at(&after, 0).as_deref(), Some("hb-1"));
    assert_eq!(
        declared_goals(&after),
        vec!["把周报写了".to_string()],
        "升格之后才算声明过（心跳据此不再重开）"
    );
}

/// **升格靠溯源认，不靠 goal 字符串相等**：同目标但来源别处的任务不得算作这条欲的产物。
#[test]
fn promotion_is_recognised_by_provenance_not_by_goal_text() {
    let want = expressed("want-0", "整理全年归档", 1, 100);
    // 同目标、但溯源指向别处（比如别的机制开的任务）。
    let other = opened_task("o-other", "t-other", "整理全年归档", 77, 150);
    let by_text = intents(&[want.clone(), other], i64::MAX);
    assert_eq!(
        task_id_at(&by_text, 0),
        None,
        "goal 字符串相等不等于这条欲升格了"
    );
    assert!(declared_goals(&by_text).is_empty());

    // 真正由这条欲长出来的任务：produced_by = 欲 seq。
    let mine = opened_task("o-1", "hb-1", "整理全年归档", 1, 200);
    let by_provenance = intents(&[want, mine], i64::MAX);
    assert_eq!(task_id_at(&by_provenance, 0).as_deref(), Some("hb-1"));
    assert_eq!(
        declared_goals(&by_provenance),
        vec!["整理全年归档".to_string()]
    );
}

/// as-of：`ts > now` 的欲与升格一概不见（与其余视图同口径）。
#[test]
fn as_of_hides_intents_and_promotions_from_the_future() {
    let events = [
        expressed("want-0", "跨天推进", 1, 1_000),
        opened_task("o-1", "hb-1", "跨天推进", 1, 2_000),
    ];

    let early = intents(&events, 500);
    assert_eq!(
        early["intents"].as_array().unwrap().len(),
        0,
        "as-of 早于表达"
    );

    let mid = intents(&events, 1_500);
    assert_eq!(mid["intents"].as_array().unwrap().len(), 1, "欲已可见");
    assert_eq!(task_id_at(&mid, 0), None, "但升格还没发生");
    assert!(declared_goals(&mid).is_empty());

    let late = intents(&events, 2_500);
    assert_eq!(declared_goals(&late), vec!["跨天推进".to_string()]);
}

/// I2 边界：无溯源的欲**进得了视图**（它是事实，藏起来才是撒谎），但永远升不了格
/// （闸门那道门先关，视图只如实报「没有派生任务」）。
#[test]
fn a_sourceless_intent_is_readable_but_never_promotes() {
    let v = intents(&[expressed("want-orphan", "来路不明", 5, 100)], i64::MAX);
    assert_eq!(v["intents"].as_array().unwrap().len(), 1, "事实不藏");
    assert_eq!(task_id_at(&v, 0), None);
    assert!(declared_goals(&v).is_empty());
}

/// 声明是**按目标**的：升格了 A 不等于声明过 B（否则去重会误伤同批的别的欲）。
#[test]
fn declaring_one_goal_does_not_declare_another() {
    let events = [
        expressed("want-a", "目标 A", 1, 100),
        expressed("want-b", "目标 B", 2, 110),
        opened_task("o-a", "hb-a", "目标 A", 1, 200),
    ];
    let v = intents(&events, i64::MAX);
    assert_eq!(
        v["intents"].as_array().unwrap().len(),
        2,
        "两条欲都在视图里——欲是流，不合并"
    );
    assert_eq!(declared_goals(&v), vec!["目标 A".to_string()]);
}

/// 采纳率 = 已升格 / 已表达。分母含「表达过但没升格」的欲——被否决 / 被忽略也是事实，不藏。
#[test]
fn adoption_rate_counts_unpromoted_intents_in_the_denominator() {
    let events = [
        expressed("want-a", "目标 A", 1, 100),
        expressed("want-b", "目标 B", 2, 110),
        expressed("want-c", "目标 C", 3, 120),
        opened_task("o-a", "hb-a", "目标 A", 1, 200),
    ];
    let v = intents(&events, i64::MAX);
    assert_eq!(v["expressed"], 3, "分母 = 全部已表达");
    assert_eq!(v["escalated"], 1, "分子 = 已升格");
    let rate = v["adoption_rate"].as_f64().unwrap();
    assert!((rate - 1.0 / 3.0).abs() < 1e-9);

    // 空视图 ⇒ 有据的零（没主动发起过，不是「采纳率未知」）
    assert_eq!(intents(&[], i64::MAX)["adoption_rate"], 0.0);
}

/// **`D-281` 的那一格**：N 片合成一个数走 owner 交的 `agg`，与单片同一条算式
/// （分子分母可加、率在汇总值上重算），不是读方再折一遍。
#[test]
fn merge_aggregates_numerator_and_denominator() {
    let s1 = intents(
        &[
            expressed("want-a", "A", 1, 10),
            expressed("want-b", "B", 2, 20),
            opened_task("o-a", "t1", "A", 1, 30),
        ],
        i64::MAX,
    );
    let s2 = intents(
        &[
            expressed("want-c", "C", 3, 40),
            opened_task("o-c", "t2", "C", 3, 50),
        ],
        i64::MAX,
    );
    let sum = merge(&[s1, s2]);
    assert_eq!(sum["expressed"], 3);
    assert_eq!(sum["escalated"], 2);
    assert!((sum["adoption_rate"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-9);
}

/// 空输入 ⇒ 有据的空（0.0），不是 0/0 的 `NaN`：读方不该因为「一个会话都没扫到」而拿到
/// 一个 `null` 或 `NaN`——那两样都会被下游渲染成「未知」，而答案是「没有」。
#[test]
fn merging_nothing_is_a_zero_not_a_division() {
    let sum = merge(&[]);
    assert_eq!(sum["expressed"], 0);
    assert_eq!(sum["escalated"], 0);
    assert_eq!(sum["adoption_rate"], 0.0);
}

/// 声明自检：读数视图都交了多片合并那一半，且 `reads` 指向已声明的名字。
///
/// 后半条不是形式主义：`reads` 里写一条没声明的名字，`seal` 会判 `UnknownFact`；写一条
/// 有事实没字段的，会判 `UndeclaredField`。两者都在装配期红，而这里让它们在**单测**里先红。
///
/// 两种合法读法：本包的事实走 `FactDecl`（名字 + 字段都校验）；兄弟插件的事实族（任务族
/// 没有 `FactDecl`）走 `D-282` 的第二种读法——名字必须在清单生成物 `SHARED_EVENTS` 里。
#[test]
fn every_view_ships_both_halves_and_its_reads_are_declared() {
    for view in VIEWS {
        assert!(
            view.agg.is_some(),
            "{}: 多片合并那一半必须交（D-281）",
            view.name
        );
        for (fact, field) in view.reads {
            if let Some(decl) = FACTS.iter().find(|f| f.name == *fact) {
                assert!(
                    decl.fields.contains(field),
                    "{}: reads 里的 {fact}.{field} 没有在 FactDecl.fields 里声明",
                    view.name
                );
            } else {
                assert!(
                    symbio_plugin_api::events::SHARED_EVENTS
                        .iter()
                        .any(|e| e.name == *fact),
                    "{}: reads 里的 {fact} 既不是本包事实，也不在共享事件表（D-282）",
                    view.name
                );
            }
        }
        assert_eq!(
            view.cost, COST,
            "{}: 扫描上界与全仓同一个值（seal 的 ViewCostMismatch：切点只有一个）",
            view.name
        );
    }
}

// ==================== 自主程度列的报表登记（D-387） ====================

fn report_ctx() -> symbio_plugin_api::reports::ReportCtx<'static> {
    symbio_plugin_api::reports::ReportCtx {
        principal: "session/main",
        cap: 0,
        readable: true,
    }
}

/// `slot = autonomy` 已登记：带跨会话轴，出口就是视图值**本身**（不加键、不改名）。
#[test]
fn the_autonomy_report_hands_back_the_view_value_untouched() {
    let reports = symbio_plugin_api::reports::reports().expect("登记表应合法");
    let report = reports
        .iter()
        .find(|r| r.slot == "autonomy")
        .expect("自主程度列应有登记项");
    assert_eq!(report.owner, "conation");
    assert_eq!(report.views, &[CONATION_AUTONOMY][..], "视图只有一个");
    assert_eq!(
        report.fleet,
        Some("axes.autonomy"),
        "跨会话那一格由本包的 agg 合并（D-281）"
    );

    let sample = json!({ "closed": 3, "unattended": 2, "unattended_rate": 2.0 / 3.0 });
    assert_eq!(
        autonomy_value(std::slice::from_ref(&sample), &report_ctx()),
        sample,
        "单片：owner 的值原样交，读方不加键"
    );
    assert_eq!(
        autonomy_value(&[], &report_ctx()),
        Value::Null,
        "读不到 ⇒ null（零是读数，null 是读不出来）"
    );
}

/// 主动性列：出口只有历史形状的两格；`declared` 与两个计数是本包的中间量，不下发。
#[test]
fn the_intents_report_drops_the_owners_intermediate_cells() {
    let reports = symbio_plugin_api::reports::reports().expect("登记表应合法");
    let report = reports
        .iter()
        .find(|r| r.slot == "conation")
        .expect("主动性列应有登记项");
    assert_eq!(report.owner, "conation");
    assert_eq!(report.views, &[CONATION_INTENTS][..], "视图只有一个");
    assert_eq!(report.fleet, Some("axes.conation"));

    let full = json!({
        "intents": [{ "seq": 1, "ts": 0, "goal": "补撤回的重试", "task_id": null }],
        "declared": ["补撤回的重试"],
        "expressed": 1,
        "escalated": 0,
        "adoption_rate": 0.0,
    });
    let out = intents_report(std::slice::from_ref(&full), &report_ctx());
    assert_eq!(out["intents"], full["intents"], "意图流原样交");
    assert_eq!(out["adoption_rate"], 0.0);
    for middle in ["declared", "expressed", "escalated"] {
        assert!(
            out.get(middle).is_none(),
            "{middle} 是本包判、本包合并用的中间量，不下发"
        );
    }

    assert_eq!(
        intents_report(&[], &report_ctx()),
        json!({ "intents": [], "adoption_rate": 0.0 }),
        "读不到 ⇒ 文档写过的空形状"
    );

    // 实例级那一格是 `agg` 之后的值本身（两格计数 + 率），与单片列不同形是有意的：
    // 跨会话没有统一的意图流，那一格只报可加的量。
    let merged = json!({ "expressed": 2, "escalated": 1, "adoption_rate": 0.5 });
    assert_eq!(
        intents_fleet(std::slice::from_ref(&merged), &report_ctx()),
        merged
    );
}
