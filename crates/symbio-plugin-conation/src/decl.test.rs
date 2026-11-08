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
use symbio_plugin_api::Seq;

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
///
/// `conation.policy` 是唯一的例外：它是**一次判决**，没有「N 片合起来该不该升格」这种东西，
/// 所以 `agg` 明确为空（`D-384`）——`NotAggregable` 是可辨的缺席，不是「汇总下来为零」。
#[test]
fn every_view_ships_both_halves_and_its_reads_are_declared() {
    for view in VIEWS {
        if view.name == CONATION_POLICY {
            assert!(
                view.agg.is_none(),
                "{}: 判决逐会话，不交合并那一半（D-384）",
                view.name
            );
        } else {
            assert!(
                view.agg.is_some(),
                "{}: 多片合并那一半必须交（D-281）",
                view.name
            );
        }
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

// ==================== `conation.policy`：开关与宽度（`D-384`） ====================
//
// 这三条口径原先躺在会话的 `ConationPolicy` 里，读方替能力包记着一条它自己的规则。
// 搬回这里之后，判据的**数字**与**边界**都归本包；会话只剩强制点（授权 + 不可伪造的
// 批准类型），因此这几条用例也只能住在这里。

use symbio_plugin_api::events::EVENT_SESSION_CONFIG;

/// 一条配置快照（`session.config`，owner 是 `loop` 的词汇表）。
fn config(seq: u64, ts: i64, payload: Value) -> Event {
    let mut e = Event::pending(
        "cfg",
        EVENT_SESSION_CONFIG,
        Entity::Scope,
        Verb::Asserted,
        "session/assembly",
    )
    .with_ts(ts)
    .with_payload(payload);
    e.seq = Some(Seq::new(seq));
    e
}

/// 带溯源的欲——生产路径（`express_intent`）的唯一形状。
fn sourced(id: &str, goal: &str, seq: u64, ts: i64) -> Event {
    expressed(id, goal, seq, ts).with_produced_by(seq - 1)
}

/// 开关：判决策读**最近一条** `session.config`；关着就拒，理由原样交给会话。
#[test]
fn policy_refuses_when_the_switch_is_off() {
    let events = [
        config(1, 10, json!({ "conation_enabled": true })),
        config(2, 20, json!({ "conation_enabled": false })),
        sourced("want-0", "把周报写了", 3, 30),
    ];
    let v = policy(&events, i64::MAX);
    assert_eq!(v["seq"], 3);
    assert_eq!(v["enabled"], false);
    assert_eq!(v["promotable"], false);
    assert_eq!(v["reason"], "conation disabled");
}

/// 缺格 / 非布尔 ⇒ 出厂值关：读不到不等于开着（`D-252` / `D-379`）。
#[test]
fn policy_defaults_the_switch_to_off() {
    let missing = policy(
        &[config(1, 10, json!({})), sourced("want-0", "看一眼", 2, 20)],
        i64::MAX,
    );
    assert_eq!(missing["enabled"], false);
    assert_eq!(missing["reason"], "conation disabled");

    let wrong_type = policy(
        &[
            config(1, 10, json!({ "conation_enabled": "true" })),
            sourced("want-0", "看一眼", 2, 20),
        ],
        i64::MAX,
    );
    assert_eq!(wrong_type["enabled"], false, "字符串 \"true\" 不是布尔");
}

/// 宽度闸：**超过** 200 字节才拒——正好 200 是允许的（边界值不许两边都算越界）。
#[test]
fn policy_refuses_only_a_goal_wider_than_the_owner_limit() {
    let open = json!({ "conation_enabled": true });
    let at_limit = "x".repeat(MAX_GOAL_LEN);
    let over = "x".repeat(MAX_GOAL_LEN + 1);

    let ok = policy(
        &[
            config(1, 10, open.clone()),
            sourced("want-0", &at_limit, 2, 20),
        ],
        i64::MAX,
    );
    assert_eq!(ok["enabled"], true);
    assert_eq!(ok["promotable"], true);
    assert_eq!(ok["reason"], Value::Null);

    let no = policy(
        &[config(1, 10, open), sourced("want-0", &over, 2, 20)],
        i64::MAX,
    );
    assert_eq!(no["promotable"], false);
    assert_eq!(no["reason"], "goal too broad");
}

/// 判决只判**带溯源**的那一条（无溯源的欲构造不出候选，I2），且 as-of 之外一概不可见。
#[test]
fn policy_binds_the_latest_sourced_intent_as_of_now() {
    let events = [
        config(1, 10, json!({ "conation_enabled": true })),
        sourced("want-a", "目标 A", 2, 20),
        expressed("want-orphan", "来路不明", 3, 30),
        config(4, 40, json!({ "conation_enabled": false })),
        sourced("want-b", "目标 B", 5, 50),
    ];
    // as-of 25：只有 A 与开着的快照 ⇒ 判 A、放行。
    let early = policy(&events, 25);
    assert_eq!(early["seq"], 2);
    assert_eq!(early["goal"], "目标 A");
    assert_eq!(early["promotable"], true);

    // as-of 45：无溯源的孤儿更晚，但它不是判决对象；开关已翻成关 ⇒ 仍判 A、拒。
    let mid = policy(&events, 45);
    assert_eq!(mid["seq"], 2);
    assert_eq!(mid["promotable"], false);
    assert_eq!(mid["reason"], "conation disabled");

    // as-of 最晚：B 是最后一条带溯源的欲 ⇒ 判 B。
    let late = policy(&events, i64::MAX);
    assert_eq!(late["seq"], 5);
    assert_eq!(late["goal"], "目标 B");
}

/// 没有可判的欲 ⇒ 明说「没有」，不拿空目标冒充一条可升格的欲（`""` 会永远通过宽度闸）。
#[test]
fn policy_without_an_intent_says_so_instead_of_guessing() {
    let v = policy(
        &[config(1, 10, json!({ "conation_enabled": true }))],
        i64::MAX,
    );
    assert_eq!(v["seq"], Value::Null);
    assert_eq!(v["promotable"], false);
    assert_eq!(v["reason"], "no expressed intent");
}
