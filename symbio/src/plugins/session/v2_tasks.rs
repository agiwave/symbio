//! 任务表写方（S7 步 16–18，[04 §3.1 批⑨](../../../../docs/plan/04-工程落地.md)）：
//! **会话任务清单 → v2 事实源 → 两个消费方**（读出口 `session/stats` 的 `readyset`
//! 列 + 交给执行者的调度段）。
//!
//! 与 `super::v2_memory`、`super::v2_bridge` 同族：任务不是另开一份存储，任务就是
//! `v2-events.wal` 里的 `task.*` 事件（ADR-044 同族纪律）。清单本身（
//! `local/todo_write`）是**纯内存、会话作用域**的（见其模块头）；事实源是同一份
//! 状态的持久化，两处各算一次就是两套真相——所以本模块只**转写观察到的声明**，
//! 不自己维护任务图。
//!
//! ## 两处消费方（一个事实一个出口）
//!
//! | 消费方 | 形态 | 门控 |
//! |---|---|---|
//! | 读出口 `session/stats` | `readyset` 列（与四列同款：事实源读什么算什么） | `may_read` |
//! | 调度段 [`prompt_section`] | 请求视图**置顶**一段 `role=user` 的提示 | 有任务 ∧ 就绪集非空 ∧ `tool_rounds == 0` |
//!
//! 投影给的是**候选集**（`readyset` 是纯函数，同一份切片永远算出同一个集合），
//! 选哪一个是调用方的策略——调度段把候选集交给本轮的执行者（模型），不替它决定
//! （`projection::readyset` 模块头的边界：优先级 / 公平性 / 饥饿都是打分参数）。
//!
//! ## 六条口径（为什么是这样）
//!
//! - **只记声明，不记全量**：`merge = true` 是增量，`merge = false` 是整体替换——
//!   本模块两种模式**行为相同**，因为事实源只认被声明到的项。清单里没提到的任务
//!   一律不动。
//! - **整体替换移出清单的项保持未终态**：append-only 不替模型编一条「已验收」的
//!   假事实。代价写在明处——它仍留在事实源里、也就仍可能留在就绪集里，读出口会
//!   念出一个清单上已经没有的 id。要表达「不再做」得先有一格 cancel 语义的事件
//!   （缺口，随后续批次补），本批不替模型决定。
//! - **降级不产事实**（进行中 → 未开始）：`task.progress` 是不可撤销的推进记录，
//!   清单把它改回 `pending` 只是模型改了主意，没有新事实发生。既不写事件、也不改
//!   观察状态，跨轮因此稳定（不会每次声明都补一条）。
//! - **回退出终态 = 返工**：`completed → pending / in_progress` 写
//!   `task.rework_created` + 一个**新节点**的 `task.opened`（`{id}-r{n}`），
//!   旧节点仍是终态——返工是新增一条事实，不是回滚
//!   （[roadmap/S03 §2](../../../../docs/plan/roadmap/S03-多步任务与返工.md)）。
//!   这正是 `rework_bounded` 的**生产写方**：不是一条永远空转的断言。
//! - **溯源锚**：全部锚在本轮 `user.message` 格（`user_seq`）——任务是「这轮模型
//!   说该做什么」，出处是这一轮的开口；`task.asserted` 是断言类事件，无溯源即违规
//!   （I2）。事件号 `{id_prefix}-{n}` 带 `{user_id}-a{attempt}` 前缀 + 本次序号，
//!   跨轮不撞幂等键。
//! - **档位**：与记忆 / 承诺同一条口径——挂在收束转写 `v2_bridge::record` 上，
//!   因此 **bridge 档生效、full 档随 full 启用**（full 档不经收束转写，其写方要
//!   等 v2 运行器原生记任务格时才接）。
//!
//! ## 诚实划界
//!
//! - **悬空依赖是断言的本职**：模型声明了一个不存在的依赖，`readyset` 会诚实地
//!   认为它不就绪，`acyclic_deps` 会在 `invariants` 列报出来。`depends_on` 默认
//!   `[]`，日常调用不踩这条线。
//! - **返工节点继承被返工任务的依赖**（返工做的是同一件事，前置条件不变）；
//!   而**别的任务**对旧 id 的依赖看的是旧节点的终态——旧节点验收过一次是事实，
//!   返工不追溯推翻它（要追溯得有「撤销终态」这一格，append-only 没有）。

use std::collections::BTreeMap;
use std::path::Path;

use crate::symbio_core::{
    Budget, Entity, Event, EventWalStore, Seq, Store, Verb, EVENT_TASK_ASSERTED, EVENT_TASK_OPENED,
    EVENT_TASK_PROGRESS, EVENT_TASK_REWORK_CREATED,
};

use super::tools::{TaskDeclaration, TaskItem, TaskStatus};

/// 一次要补进事实源的事件（算完再写：状态推进与写入分开，便于单测断言「该补几条」）。
struct Emitted {
    kind: &'static str,
    verb: Verb,
    payload: serde_json::Value,
}

/// 清单 id → 观察状态（**从事件序列重建**，不跨轮持有内存记账——同一份切片
/// 必须永远算出同一套状态，否则桥档与 full 档、单测与生产会各走一套）。
#[derive(Debug, Default, Clone)]
struct Entry {
    /// 该清单 id 当前对应的**存活节点**（返工后是 `{id}-r{n}`）。
    live: String,
    /// 存活节点的状态。
    status: TaskStatus,
    /// 存活节点的依赖（返工节点继承它）。
    depends_on: Vec<String>,
    /// 存活节点的任务内容。
    goal: String,
    /// 已发生的返工轮数。
    rounds: u32,
}

/// 载荷里的字符串字段（缺省 = 空串 / 空表，与 `readyset` 的读法同口径）。
fn payload_str(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn payload_deps(payload: &serde_json::Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// 从已有事实重建观察状态。
///
/// 返工节点用 `task.rework_created` 载荷的 `replaces` 归到其清单项上：只有那一格
/// 携带 `replaces`，所以先登记「节点 id → 清单 id」，此后落在返工节点上的
/// opened / progressed / asserted 都按这张表归位——否则返工会在表里开出第二个条目，
/// 同一个清单项被记成两件事。
fn rebuild(snapshot: &[Event]) -> BTreeMap<String, Entry> {
    let mut map: BTreeMap<String, Entry> = BTreeMap::new();
    let mut nodes: BTreeMap<String, String> = BTreeMap::new();
    for e in snapshot {
        if e.entity != Entity::Task {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if e.kind == EVENT_TASK_REWORK_CREATED {
            let replaces = payload_str(&e.payload, "replaces");
            if replaces.is_empty() {
                continue;
            }
            nodes.insert(id.to_string(), replaces.clone());
            let round = e
                .payload
                .get("round")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1) as u32;
            let entry = map.entry(replaces).or_default();
            entry.rounds = entry.rounds.max(round);
            continue;
        }
        let key = nodes.get(id).map(String::as_str).unwrap_or(id);
        match e.kind.as_str() {
            k if k == EVENT_TASK_OPENED => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::Pending;
                entry.depends_on = payload_deps(&e.payload, "depends_on");
                entry.goal = payload_str(&e.payload, "goal");
            }
            k if k == EVENT_TASK_PROGRESS => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::InProgress;
            }
            k if k == EVENT_TASK_ASSERTED => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::Completed;
            }
            _ => {}
        }
    }
    map
}

/// 一次声明该补的事件（状态推进的**判定方**：只看观察状态 + 声明态，不看清单全量）。
fn step(entry: &mut Entry, list_id: &str, item: &TaskItem) -> Vec<Emitted> {
    let mut out = Vec::new();
    if entry.live.is_empty() {
        // 第一次见到这个清单 id：先开格（`readyset` 只在 `task.opened` 建节点，
        // 没有这一格的任务根本不在调度的候选集里）。
        entry.live = list_id.to_string();
        entry.status = TaskStatus::Pending;
        entry.depends_on = item.depends_on.clone();
        entry.goal = item.goal.clone();
        out.push(Emitted {
            kind: EVENT_TASK_OPENED,
            verb: Verb::Opened,
            payload: serde_json::json!({
                "task_id": list_id,
                "depends_on": item.depends_on,
                "goal": item.goal,
            }),
        });
    }

    // 降级（进行中 → 未开始）不产事实：见模块头「降级不产事实」。
    if item.status == TaskStatus::Pending {
        return out;
    }

    if entry.status == TaskStatus::Completed && item.status != TaskStatus::Completed {
        // 回退出终态 = 返工：新增一条判定事实 + 一个新节点，旧节点仍是终态。
        entry.rounds += 1;
        let node = format!("{list_id}-r{}", entry.rounds);
        out.push(Emitted {
            kind: EVENT_TASK_REWORK_CREATED,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "task_id": node,
                "replaces": list_id,
                "round": entry.rounds,
            }),
        });
        out.push(Emitted {
            kind: EVENT_TASK_OPENED,
            verb: Verb::Opened,
            payload: serde_json::json!({
                "task_id": node,
                "depends_on": entry.depends_on,
                "goal": entry.goal,
            }),
        });
        entry.live = node;
        entry.status = TaskStatus::Pending;
    }

    if entry.status != item.status {
        match item.status {
            TaskStatus::InProgress => {
                out.push(Emitted {
                    kind: EVENT_TASK_PROGRESS,
                    verb: Verb::Progressed,
                    payload: serde_json::json!({ "task_id": entry.live }),
                });
                entry.status = TaskStatus::InProgress;
            }
            TaskStatus::Completed => {
                out.push(Emitted {
                    kind: EVENT_TASK_ASSERTED,
                    verb: Verb::Asserted,
                    payload: serde_json::json!({ "task_id": entry.live }),
                });
                entry.status = TaskStatus::Completed;
            }
            // `Pending` 已在上面提前返回。
            TaskStatus::Pending => {}
        }
    }
    out
}

/// 把本轮的 `todo_write` 声明写进事实源（收束转写的第三个写方，批⑨）。
///
/// - `produced_by` = 本轮 `user.message` 的 seq（溯源锚，I2）；
/// - `id_prefix` = `v2t-{user_id}-a{attempt}`，事件号是 `{id_prefix}-{n}`；
/// - 失败冒泡给调用方记日志（与记忆三段同款：附加事实失败不算转写失败）。
pub(crate) fn write(
    store: &EventWalStore,
    snapshot: &[Event],
    decls: &[TaskDeclaration],
    turn: u64,
    produced_by: u64,
    principal: &str,
    id_prefix: &str,
) -> Result<(), String> {
    if decls.is_empty() {
        return Ok(());
    }
    let mut map = rebuild(snapshot);
    let mut n = 0usize;
    for decl in decls {
        for item in &decl.items {
            for e in step(map.entry(item.id.clone()).or_default(), &item.id, item) {
                let event_id = format!("{id_prefix}-{n}");
                n += 1;
                store
                    .append(
                        Event::pending(event_id, e.kind, Entity::Task, e.verb, turn, principal)
                            .with_produced_by(produced_by)
                            .with_payload(e.payload),
                    )
                    .map_err(|err| format!("任务事件入格失败：{err:?}"))?;
            }
        }
    }
    Ok(())
}

/// 调度段（S7 步 17）：就绪任务集渲染成一段提示，请求视图**置顶**注入。
///
/// 只读不写（与 `v2_memory::recall_view` 同款的只读视图），因此：
/// - 文件不存在 / 读不了 ⇒ `None`——请求里就少一段，不存在「空占位」；
/// - 就绪集空 ⇒ `None`——依赖没闭合时给模型一个空的候选集只会诱导它瞎猜。
///
/// 调用方（`chat_loop::inputs`）按 `tool_rounds == 0` 取一次、本轮各工具轮复用。
pub(crate) fn prompt_section(session_dir: &Path) -> Option<String> {
    let path = session_dir.join(super::paths::V2_WAL_FILE);
    // 只读打开：读方不创建文件、不截尾（`wal.rs::open_readonly`）。
    let store = EventWalStore::open_readonly(&path).ok()?;
    let snapshot = store.range(Seq::new(0));
    let ready = crate::symbio_core::readyset()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value
        .ready;
    if ready.is_empty() {
        return None;
    }
    let ids = ready
        .iter()
        .map(|t| t.task_id.as_str())
        .collect::<Vec<_>>()
        .join("、");
    Some(format!(
        "【任务调度】就绪任务（依赖已全部完成）：{ids}。请从就绪任务中推进一项；\
         其余任务的依赖尚未完成，暂不可开始。"
    ))
}

#[cfg(test)]
#[path = "v2_tasks.test.rs"]
mod tests;
