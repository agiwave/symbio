//! `conation.autonomy` 视图：无人干预完成率（[DE-008 §3.1 外部效标 · 自主程度](../../../docs/003-design/008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系")）。
//!
//! 一条任务闭环（`task.asserted`）之前，**是否从未被挂起过**（`task.held`）。
//! `task.held` 的写方是插话抢占（`crates/symbio-plugin-session` 的收件箱）——它是
//! **外部介入**这个事实在事实源里唯一的痕迹：人或外部条件改变了任务原本的走向。故：
//!
//! ```text
//! 无人干预完成率 = 闭环且全程未 held 的任务数 / 闭环任务数
//! ```
//!
//! ## 为什么住本包
//!
//! 它与 `conation.intents` 是**同一条轴的两个面**（DE-008 §3.1 外部效标）：主动性问
//! 「自己发起了多少」，自主程度问「发起之后有多少独立走完」。两个折叠的读者都是
//! `session` 的统计出口，住同一个包才不会让「自主层读数」跨两个 crate。
//!
//! ## 拼写与读法的两个边界
//!
//! - 任务族五格的拼写取自清单生成物 `symbio_plugin_api::events`（owner 是
//!   `crates/symbio-plugin-task`），不引兄弟插件（E-009）；
//! - 任务族只有 `EventDecl`、没有 `FactDecl`，故 `reads` 按 `D-282` 的第二种读法登记
//!   名字（与 `task.readyset` 同形）：名字由声明校验，字段无从校验。
//!
//! ## 为什么用「曾 held」而不是「当前 held」
//!
//! `task.progress` 会解除挂起（任务回到就绪），但**挂起这件事已经发生了**——
//! 系统确实没能独立走完。用「当前 held」会把「被干预过又恢复」读成「全程自主」，
//! 那是把干预洗掉。此处取保守口径：干预过一次就算干预过。
//!
//! ## 为什么分子分母只数已闭环的
//!
//! 在跑的任务还没给出结论。把它们算进分母会让读数随「有多少任务正在跑」漂移——
//! 那不是自主程度变了，是并发度变了。未闭环的任务**既不进分子也不进分母**
//! （与 `slo.span` 的「一个事件的会话不进分母」同一条纪律：不拿「还没发生」冒充
//! 「发生了且为零」）。
//!
//! ## 边界（诚实划界）
//!
//! - **只认 `task.held` 这一种干预**：用户改需求、外部接口超时等若没有走 `held`
//!   事件，本折叠看不见。它是「抢占式干预」的下界，不是全部干预的上界；
//! - **不判任务质量**：闭环了就算完成（判质量是 `task.rework_created` 与返工机制
//!   的事）。

use serde_json::{json, Value};
use symbio_plugin_api::events::{EVENT_TASK_ASSERTED, EVENT_TASK_HELD};
use symbio_plugin_api::{Entity, Event};

/// 无人干预完成率 ∈ [0, 1]。
///
/// 无已闭环任务 ⇒ `0.0`（**有据的零**：分母为空时率无意义，给 0 而不是 NaN——
/// 序列化里没有 NaN，且「没有完成过任务」不该读成「自主完成率未知」）。
fn rate(closed: u64, unattended: u64) -> f64 {
    if closed == 0 {
        return 0.0;
    }
    unattended as f64 / closed as f64
}

/// 纯折叠：`task.held` / `task.asserted` → 分子分母 + 率（as-of：`ts > now` 一概不见）。
///
/// 确定性（N1）：同一事件序列 ⇒ 逐字节相同的 `Value`。按任务 id 去重（计数不按事件数）。
pub(crate) fn autonomy(events: &[Event], now: i64) -> Value {
    let mut held: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut closed: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for e in events {
        if e.ts > now || e.entity != Entity::Task {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(Value::as_str) else {
            continue;
        };
        if e.kind.as_str() == EVENT_TASK_HELD {
            held.insert(id);
        } else if e.kind.as_str() == EVENT_TASK_ASSERTED {
            closed.insert(id);
        }
    }
    let closed_n = closed.len() as u64;
    // 分子 = 闭环 ∧ 未挂起（曾 held 过的排除——干预过一次就算干预过）
    let unattended_n = closed.iter().filter(|id| !held.contains(**id)).count() as u64;
    json!({
        "closed": closed_n,
        "unattended": unattended_n,
        "unattended_rate": rate(closed_n, unattended_n),
    })
}

/// **跨会话合并**：N 份 [`autonomy`] 的产物 → 一份汇总（`D-281`）。
///
/// 只收分子 / 分母，率由 [`rate`] 从汇总值重算——合并的是**可加的量**，率是同一套算式
/// 在汇总值上的再应用，不是另一份口径。认不出的载荷按 0 计：形状是上一版交的，
/// 读不出说明版本对不上——那会让率变小，而不会让率凭空出现（`D-281` 理由 3）。
pub(crate) fn merge(parts: &[Value]) -> Value {
    let mut closed = 0u64;
    let mut unattended = 0u64;
    for p in parts {
        closed += p.get("closed").and_then(Value::as_u64).unwrap_or(0);
        unattended += p.get("unattended").and_then(Value::as_u64).unwrap_or(0);
    }
    json!({
        "closed": closed,
        "unattended": unattended,
        "unattended_rate": rate(closed, unattended),
    })
}

#[cfg(test)]
#[path = "autonomy.test.rs"]
mod tests;
