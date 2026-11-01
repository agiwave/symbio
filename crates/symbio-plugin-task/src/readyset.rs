//! 就绪集折叠（名册 #23「凸显仲裁」，[DE-033 §3](../../../docs/003-design/033-多步任务与返工.md "DE-033-多步任务与返工")、
//! [DE-034](../../../docs/003-design/034-并发调度与租约.md "DE-034-并发调度与租约")）。
//!
//! 这一条折叠以前住在 core 的投影目录里，理由是「读者 ≥2 个插件」。今天它的写方、调度者
//! 与读数出口分别是本包、本包与 `session`，而 `session` 改成了按名字读（`D-221` / `D-289`）
//! ⇒ 折叠跟着 owner 走，core 那一格交还。
//!
//! **就绪判定是纯函数，从事件序列算**：`ready = 非终态 ∧ depends_on 全部终态 ∧ 无活租约 ∧
//! 未挂起`。调度器不需要任何内部状态——同一份事件切片永远算出同一个就绪集（N1）。
//!
//! ## 容量是读者的预算，不是折叠的一格
//!
//! [`ready_all`] 交出**全部**就绪任务，按 `(opened_seq, task_id)` 升序；`cap` 由读者在
//! 折叠之外套上（[`ready_view`] 那一层）。这不是把口径拆成两份：折叠回答「谁就绪、什么序」，
//! 读者回答「这一轮给几笔」——后者本来就是参数而不是结构（`cap = 1` 是本阶的平凡值，
//! 调度器、租约、仲裁的逻辑一行不改，系统退化为串行 FIFO，
//! [DE-034](../../../docs/003-design/034-并发调度与租约.md "DE-034-并发调度与租约") §4）。
//! 搬出 core 之后这件事第一次**写在类型上**：声明视图的求值口只有事实切片与时间戳
//! （`ViewDecl::eval`），容量根本没有进来的通道 ⇒ 谁想截断都得自己说明截了几笔。
//!
//! `cap = 0` 按 1 处理：容量为零的调度器不是「更保守」，是「永不动」。
//!
//! ## 租约：持有状态由折叠算，不是锁
//!
//! `task.assigned` 带 `until_ms`（租约到期时刻）。**未到期的租约让任务离开就绪集**
//! ——被派出去的活不该被第二次派出去。到期即失效：那一刻起任务重新是候选，判死的
//! 事实由调度者补落（`task.ready{reason: "lease_expired"}`），折叠只认时间戳，
//! 不自己写事件。
//!
//! 租约的失效**不看** `owner` 是否还活着：活着的判断需要心跳，而心跳是另一条事实
//! （见 DE-034 §8 的边界）。到期是唯一判据，它足够安全——早到期只让任务更早回到
//! 候选集，晚到期的代价被 `lease_ms` 约束。
//!
//! ## 挂起排除（DE-037 §5 的强制点）
//!
//! `task.held` 的任务不进就绪集——被挂起的任务被二次调度会重复执行；恢复（同任务的
//! `task.progress`）自动解除挂起。挂起是一条事件，恢复也是，折叠不持有「现在挂没挂」
//! 这个状态。
//!
//! ## 边界（DE-033/DE-034 §7 诚实划界）
//!
//! - 优先级、公平性、饥饿——都是就绪集的**打分参数**，不是机制；选什么打分函数是算法
//!   问题，架构不保证（本折叠按 `(opened_seq, task_id)`，确定性优先）；
//! - 环与悬空依赖不在这里判——那是 C14（core 的 `invariants::acyclic_deps`）的职责。
//!
//! ## 这一族的五个名字
//!
//! 建节点 / 推进 / 挂起 / 终态 / 租约是**算法角色**，拼写在 [`crate::decl::TASK_VOCAB`]。
//! 住在 core 的那些年里这张表得在构造期当形参交进来（DE-024 D-206），因为 core 不认识
//! 任务这个词；owner 自己认自己的名字，形参因此退场——`session` 的 `autonomy` 折叠还留在
//! 它那一份同源拼写上（它是另一条折叠，见 D-206 的现存理由）。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use symbio_plugin_api::{Entity, Event};

use crate::decl::TASK_VOCAB;

/// 一条就绪任务。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ReadyTask {
    /// 任务 id（`task.opened` 载荷的 `task_id`）。
    pub(crate) task_id: String,
    /// 开任务事件的 seq（调度器取事件的定位锚）。
    pub(crate) seq: u64,
    /// 依赖（已全部终态——放进就绪集时必然满足）。
    pub(crate) depends_on: Vec<String>,
}

/// 就绪集视图：按 `(opened_seq, task_id)` 升序，至多 `cap` 笔。
///
/// `cap` 写进视图，是因为「为什么只给这几笔」是读方要回答的问题：候选集被截断与
/// 候选集本来就少，是两种不同的信号。它是**读者那一层**的记账（[`ready_view`]），
/// 不在对外声明的视图值里（见模块文档「容量是读者的预算」）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct ReadySetView {
    pub(crate) ready: Vec<ReadyTask>,
    pub(crate) cap: usize,
}

/// 任务族的一格在折叠算法里的角色（名字比对只做这一次，且只在这五格里做）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskCell {
    Opened,
    Progressed,
    Held,
    Terminal,
    Lease,
}

/// 认名字：不认识的那一格返回 `None`，由折叠显式跳过（`task.ready` / `task.rework_created`
/// 也在 `Task` 实体上，它们不参与就绪判定）。
fn cell(kind: &str) -> Option<TaskCell> {
    if kind == TASK_VOCAB.opened {
        Some(TaskCell::Opened)
    } else if kind == TASK_VOCAB.progressed {
        Some(TaskCell::Progressed)
    } else if kind == TASK_VOCAB.held {
        Some(TaskCell::Held)
    } else if kind == TASK_VOCAB.terminal {
        Some(TaskCell::Terminal)
    } else if kind == TASK_VOCAB.lease {
        Some(TaskCell::Lease)
    } else {
        None
    }
}

/// 租约记账的一笔：`until_ms` 之前这笔任务都算「有人在做」。
#[derive(Debug, Clone, Copy)]
struct Lease {
    until_ms: i64,
}

/// 就绪集折叠本体：**全部**就绪任务，按入队序，不设容量。
///
/// - 终态 = `terminal`（验收通过才终态——返工节点重开后不算旧任务复活）；
/// - `ts > now` 的事件不参与（与其余折叠同口径）；
/// - **挂起排除**：`held` 的任务不进就绪集；恢复（同任务的 `progressed`）自动解除挂起；
/// - **活租约排除**：`lease` 且 `until_ms > now` 的任务不进就绪集；租约到期或任务终态
///   即释放；缺 `until_ms` 的租约按「没有租约」处理，而不是「永久持有」——宁可重复派
///   一次，不要永远不派；
/// - 排序：`BTreeMap` 按 id 字典序收集，再按 `(opened_seq, task_id)` 排一次——调度要的
///   是**入队序**（先开的活先派），字典序只做同 seq 的兜底。缺 `seq` 的落格记成
///   `u64::MAX`（排在最后），而不是记成 0（那会让一条无序事件抢在所有人前面）。
pub(crate) fn ready_all(events: &[Event], now: i64) -> Vec<ReadyTask> {
    // task_id → (opened 事件 seq, depends_on)
    let mut tasks: BTreeMap<&str, (u64, Vec<String>)> = BTreeMap::new();
    let mut terminal: BTreeMap<&str, ()> = BTreeMap::new();
    let mut held: BTreeMap<&str, ()> = BTreeMap::new();
    let mut leases: BTreeMap<&str, Lease> = BTreeMap::new();
    for e in events {
        if e.ts > now || e.entity != Entity::Task {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(cell) = cell(&e.kind) else {
            continue;
        };
        match cell {
            TaskCell::Opened => {
                let ds = e
                    .payload
                    .get("depends_on")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                tasks.insert(id, (e.seq.map(|s| s.value()).unwrap_or(u64::MAX), ds));
            }
            TaskCell::Terminal => {
                terminal.insert(id, ());
                held.remove(id);
                // 终态之后租约没有意义：那笔活已经交出去了。
                leases.remove(id);
            }
            TaskCell::Held => {
                // 挂起：DE-037 §5 的强制点——挂起期间不得被二次调度。
                held.insert(id, ());
            }
            TaskCell::Progressed => {
                // 恢复：挂起是一条事件，恢复也是——progressed 自动解除挂起。
                held.remove(id);
            }
            TaskCell::Lease => {
                // 租约：`until_ms` 是**到期时刻**（绝对时间，ms）。
                // 缺这一格 = 无期限租约不成立——按「没有租约」处理，
                // 而不是按「永久持有」：宁可重复派一次，不要永远不派。
                let until_ms = e
                    .payload
                    .get("until_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(i64::MIN);
                leases.insert(id, Lease { until_ms });
            }
        }
    }
    let mut ready: Vec<ReadyTask> = tasks
        .iter()
        .filter(|(id, (_, ds))| {
            !terminal.contains_key(*id)
                && !held.contains_key(*id)
                && leases.get(*id).is_none_or(|lease| lease.until_ms <= now)
                && ds.iter().all(|d| terminal.contains_key(d.as_str()))
        })
        .map(|(id, (seq, ds))| ReadyTask {
            task_id: (*id).to_string(),
            seq: *seq,
            depends_on: ds.clone(),
        })
        .collect();
    ready.sort_by(|a, b| (a.seq, &a.task_id).cmp(&(b.seq, &b.task_id)));
    ready
}

/// 读者那一层：把本轮的容量套在折叠的结果上。
///
/// `cap = 0` 按 1 处理（见模块文档）。视图里的 `cap` 因此是**生效**容量，不是调用方
/// 交的那一个——报回原值会让「配了 0」与「配了 1」在出口上读起来不一样。
pub(crate) fn ready_view(events: &[Event], now: i64, cap: usize) -> ReadySetView {
    let cap = cap.max(1);
    let mut ready = ready_all(events, now);
    ready.truncate(cap);
    ReadySetView { ready, cap }
}

/// 对外声明的那一份：`task.readyset` 的值（[`ready_all`] 的原样序列化，不含 `cap`）。
///
/// 读方拿到的是**未截断**的候选集与它们的序；「这一轮派几笔」是读者自己的事
/// （`session` 的统计列按会话配置截，本包的调度者按同一个 `task_cap` 截）。
/// 同一切片、同一个 as-of ⇒ 两侧读出的前缀必然同源（N1 双跑一致）。
pub(crate) fn readyset_value(events: &[Event], now: i64) -> Value {
    json!({ "ready": ready_all(events, now) })
}

#[cfg(test)]
#[path = "readyset.test.rs"]
mod tests;
