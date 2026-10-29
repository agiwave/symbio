//! `readyset` 投影（S7 第 16 步，[roadmap/S03 §3](../../../../docs/003-design/033-多步任务与返工.md "DE-033-多步任务与返工")、
//! [roadmap/S04](../../../../docs/003-design/034-并发调度与租约.md "DE-034-并发调度与租约")）。
//!
//! **就绪判定是纯函数，从事件序列算**：`ready = 非终态 ∧ depends_on 全部终态 ∧ 无活租约`。
//! 调度器不需要任何内部状态——同一份事件切片永远算出同一个就绪集（N1）。
//!
//! ## 容量是参数，不是结构（`cap`）
//!
//! [`readyset`] 收一个 `cap`：就绪集**最多这么多笔**，按 `(opened_seq, task_id)` 升序取前
//! `cap` 笔。`cap = 1` 是本阶的平凡值——调度器、租约、仲裁的逻辑一行不改，系统退化为
//! S03 的串行 FIFO（[DE-034](../../../../docs/003-design/034-并发调度与租约.md "DE-034-并发调度与租约") §4）。
//! `cap = 0` 按 1 处理：容量为零的调度器不是「更保守」，是「永不动」。
//!
//! ## 租约：持有状态由投影算，不是锁
//!
//! `task.assigned` 带 `until_ms`（租约到期时刻）。**未到期的租约让任务离开就绪集**
//! ——被派出去的活不该被第二次派出去。到期即失效：那一刻起任务重新是候选，判死的
//! 事实由调度者补落（`task.ready{reason: "lease_expired"}`），投影只认时间戳，
//! 不自己写事件。
//!
//! 租约的失效**不看** `owner` 是否还活着：活着的判断需要心跳，而心跳是另一条事实
//! （见 DE-034 §8 的边界）。到期是唯一判据，它足够安全——早到期只让任务更早回到
//! 候选集，晚到期的代价被 `lease_ms` 约束。
//!
//! ## 边界（S03/S04 §7 诚实划界）
//!
//! - 优先级、公平性、饥饿——都是 `readyset` 的**打分参数**，不是机制；
//!   选什么打分函数是算法问题，架构不保证（本投影按 `(opened_seq, task_id)`，确定性优先）；
//! - 环与悬空依赖不在这里判——那是 C14（[`crate::invariants::acyclic_deps`]）
//!   的职责。
//!
//! ## 这一族的五个名字不在 core 里
//!
//! 建节点 / 推进 / 挂起 / 终态 / 租约是**算法角色**；它们的拼写由调用方在构造期交进来
//! （`readyset_for(vocab, cap)`，DE-024 D-206）。
//! 写方与调度者是 `crates/symbio-plugin-loop`，读数还被 `crates/symbio-plugin-session` 的 `stats` 消费，所以名字登记进
//! loop 的清单、由生成物交给 session（D-204）——core 一行都不比对它们。

use super::super::event::{Entity, Event};
use super::task_vocab::{TaskRole, TaskVocab};
use super::Projection;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一条就绪任务。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReadyTask {
    /// 任务 id（`task.opened` 载荷的 `task_id`）。
    pub task_id: String,
    /// 开任务事件的 seq（调度器取事件的定位锚）。
    pub seq: u64,
    /// 依赖（已全部终态——放进就绪集时必然满足）。
    pub depends_on: Vec<String>,
}

/// 就绪集视图：按 `(opened_seq, task_id)` 升序，至多 `cap` 笔。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReadySetView {
    pub ready: Vec<ReadyTask>,
    /// 本视图的容量（写进视图，是因为「为什么只给这几笔」是读方要回答的问题：
    /// 候选集被截断与候选集本来就少，是两种不同的信号）。
    #[serde(default)]
    pub cap: usize,
}

/// 租约记账的一笔：`until_ms` 之前这笔任务都算「有人在做」。
#[derive(Debug, Clone, Copy)]
struct Lease {
    until_ms: i64,
}

/// `readyset` 投影：从事件序列算出当前就绪的任务（至多 `cap` 笔）。
///
/// **这一族的五条名字不写在 core 里**（DE-024 D-206）：`readyset_for` 在构造期收一张
/// [`TaskVocab`]，owner 插件交自家常量，第二个消费方交清单生成物（D-204）。下面每一格
/// 都用它的**算法角色**称呼（建节点 / 推进 / 挂起 / 终态 / 租约），因为终态与租约落在
/// 同一格 `Task × Asserted`——按格子或按事件名都说不清这五件事。
///
/// - 终态 = `terminal`（验收通过才终态——返工节点重开后不算旧任务复活）；
/// - `ts > now` 的事件不参与（与其余投影同口径）；
/// - **挂起排除**（S07 §5 强制点）：`held` 的任务不进就绪集——被挂起的任务
///   被二次调度可能重复执行；恢复（同任务的 `progressed`）自动解除挂起；
/// - **活租约排除**（S04）：`lease` 且 `until_ms > now` 的任务不进就绪集；
///   租约到期或任务终态即释放；
/// - 按 `(opened_seq, task_id)` 升序取前 `cap` 笔（`cap = 0` 按 1 处理）。
///
/// 生产消费方两处（[04 §3.1 批⑨](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：读出口
/// `session/stats` 的 `readyset` 列，与请求装配时交给执行者的**调度段**
/// （`crates/symbio-plugin-loop/src/task_events`）、以及调度者自己。两者传同一个 `cap`（写方 = 会话配置），
/// 因此「候选集多大」只有一处定义。
pub fn readyset_for(vocab: TaskVocab<'_>, cap: usize) -> Projection<ReadySetView> {
    let cap = cap.max(1);
    let vocab = vocab.bind();
    Projection::new(move |events: &[Event], now| {
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
            let Some(role) = vocab.role(&e.kind) else {
                continue;
            };
            match role {
                TaskRole::Opened => {
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
                TaskRole::Terminal => {
                    terminal.insert(id, ());
                    held.remove(id);
                    // 终态之后租约没有意义：那笔活已经交出去了。
                    leases.remove(id);
                }
                TaskRole::Held => {
                    // 挂起：S07 §5 的强制点——挂起期间不得被二次调度。
                    held.insert(id, ());
                }
                TaskRole::Progressed => {
                    // 恢复：挂起是一条事件，恢复也是——progressed 自动解除挂起。
                    held.remove(id);
                }
                TaskRole::Lease => {
                    // 租约：`until_ms` 是**到期时刻**（绝对时间，ms）。
                    // 缺这一格 = 无期限租约不成立——按「没有租约」处理，
                    // 而不是按「永久持有」：宁可重复派一次，不要永远不派。
                    let until_ms = e
                        .payload
                        .get("until_ms")
                        .and_then(serde_json::Value::as_i64)
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
        // `tasks` 是 `BTreeMap<&str, _>`（按 id 字典序），而调度要的是**入队序**：
        // 先开出来的活先派。再按 (seq, task_id) 排一次，字典序只做同 seq 的兜底。
        ready.sort_by(|a, b| (a.seq, &a.task_id).cmp(&(b.seq, &b.task_id)));
        ready.truncate(cap);
        ReadySetView { ready, cap }
    })
}
