//! `conation.autonomy` 视图的验收——无人干预完成率的口径（闭环任务中「全程未被挂起」的比例）。
//!
//! 反向用例（改输入会变红）：曾 held 又恢复的任务**不算**自主（干预过就是干预过，
//! progress 不洗掉事实）；未闭环的任务不进分母（没结论的事不得冒充分母）；
//! as-of 之后的事件一概不见；无闭环任务 ⇒ 有据的零 0.0。
//!
//! 事件直接构造、不经存储：本 crate 不链接存储（HS-001），测的是折叠，不是入库。

use super::*;
use symbio_plugin_api::Verb;

const INITIATOR: &str = "session/reflex";

fn held(id: &str, task: &str, ts: i64) -> Event {
    Event::pending(id, EVENT_TASK_HELD, Entity::Task, Verb::Held, INITIATOR)
        .with_ts(ts)
        .with_payload(json!({ "task_id": task }))
}

fn asserted(id: &str, task: &str, ts: i64) -> Event {
    Event::pending(
        id,
        EVENT_TASK_ASSERTED,
        Entity::Task,
        Verb::Asserted,
        INITIATOR,
    )
    .with_ts(ts)
    .with_payload(json!({ "task_id": task }))
}

fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(u64::MAX)
}

fn rate_of(v: &Value) -> f64 {
    v["unattended_rate"].as_f64().unwrap_or(f64::NAN)
}

/// 口径：闭环且未挂起 / 闭环。
#[test]
fn unattended_rate_is_closed_without_held_over_closed() {
    // t1 全程自主；t2 被挂起过（后面才闭环）⇒ 不进分子
    let v = autonomy(
        &[
            held("h1", "t2", 100),
            asserted("a1", "t1", 200),
            asserted("a2", "t2", 300),
        ],
        i64::MAX,
    );
    assert_eq!(count(&v, "closed"), 2, "分母 = 已闭环任务数");
    assert_eq!(count(&v, "unattended"), 1, "t2 挂起过 ⇒ 不算自主完成");
    assert!((rate_of(&v) - 0.5).abs() < 1e-9);
}

/// 反向：挂起后又 progress 恢复、最终闭环——**仍不算**无人干预。
/// 干预这件事已经发生了，恢复不把它洗掉（模块头「为什么用曾 held」）。
#[test]
fn recovery_does_not_wash_out_the_intervention() {
    let v = autonomy(
        &[
            held("h1", "t1", 100),
            Event::pending(
                "p1",
                "task.progress",
                Entity::Task,
                Verb::Progressed,
                INITIATOR,
            )
            .with_ts(150)
            .with_payload(json!({ "task_id": "t1" })),
            asserted("a1", "t1", 200),
        ],
        i64::MAX,
    );
    assert_eq!(count(&v, "closed"), 1);
    assert_eq!(count(&v, "unattended"), 0, "被干预过又恢复 ⇒ 不是全程自主");
    assert_eq!(rate_of(&v), 0.0);
}

/// 未闭环的任务（只开格 / 只挂起）**既不进分子也不进分母**——
/// 在跑的任务还没给出结论，算进去会让读数随并发度漂移。
#[test]
fn unfinished_tasks_are_excluded_from_both_sides() {
    let v = autonomy(
        &[
            Event::pending("o1", "task.opened", Entity::Task, Verb::Opened, INITIATOR)
                .with_ts(100)
                .with_payload(json!({ "task_id": "running" })),
            held("h1", "suspended", 200),
        ],
        i64::MAX,
    );
    assert_eq!(count(&v, "closed"), 0, "没有闭环 ⇒ 分母为空");
    assert_eq!(count(&v, "unattended"), 0);
    assert_eq!(rate_of(&v), 0.0, "有据的零，不是 NaN");
}

/// as-of：闭环发生在未来 ⇒ 此刻看不见（与其余视图同口径）。
#[test]
fn as_of_hides_future_assertions() {
    let v = autonomy(
        &[asserted("a1", "t1", 1_000), asserted("a2", "t2", 9_000)],
        2_000,
    );
    assert_eq!(count(&v, "closed"), 1, "as-of = 2000 ⇒ 只见第一条闭环");
    assert!((rate_of(&v) - 1.0).abs() < 1e-9);
}

/// 同一任务被挂起两次、闭环一次 ⇒ 只算一条（幂等：计数按任务去重，不按事件数）。
#[test]
fn repeated_held_on_one_task_counts_once() {
    let v = autonomy(
        &[
            held("h1", "t1", 100),
            held("h2", "t1", 150),
            asserted("a1", "t1", 200),
        ],
        i64::MAX,
    );
    assert_eq!(count(&v, "closed"), 1);
    assert_eq!(count(&v, "unattended"), 0);
}

/// 跨会话合并：分子分母可加，率是同一套算式在汇总值上的再应用。
#[test]
fn merge_adds_numerator_and_denominator() {
    let s1 = json!({ "closed": 4, "unattended": 3, "unattended_rate": 0.75 });
    let s2 = json!({ "closed": 6, "unattended": 2, "unattended_rate": 1.0 / 3.0 });
    let sum = merge(&[s1, s2]);
    assert_eq!(count(&sum, "closed"), 10);
    assert_eq!(count(&sum, "unattended"), 5);
    assert!((rate_of(&sum) - 0.5).abs() < 1e-9);
    // 空输入 ⇒ 空汇总（有据的空）
    let empty = merge(&[]);
    assert_eq!(count(&empty, "closed"), 0);
    assert_eq!(rate_of(&empty), 0.0);
}

/// 非 Task 实体 / 没有 task_id 的事件一概不认（折叠只吃自己的格子）。
#[test]
fn ignores_non_task_events() {
    let v = autonomy(
        &[
            Event::pending("x1", EVENT_TASK_HELD, Entity::Scope, Verb::Held, INITIATOR)
                .with_ts(100)
                .with_payload(json!({ "task_id": "t1" })),
            Event::pending(
                "x2",
                EVENT_TASK_ASSERTED,
                Entity::Task,
                Verb::Asserted,
                INITIATOR,
            )
            .with_ts(150)
            .with_payload(json!({ "other": 1 })),
        ],
        i64::MAX,
    );
    assert_eq!(count(&v, "closed"), 0, "实体不符 / 缺 task_id ⇒ 不计数");
}
