//! 任务表声明出参（一次**成功**的 `todo_write` 声明的清单状态）：观察点是模型工具
//! 调用的唯一入口，声明随轮末收束落成 `task.*` 格。

use serde_json::Value;

/// 任务表声明（S7 步 16–18，[04 §3.1 批⑨](../../../../../../docs/plan/04-工程落地.md)）：
/// 一次**成功**的 `todo_write` 调用所声明的清单状态。
///
/// 与 [`Delegation`](super::Delegation) 同一个观察点（[`process_tool_calls_async`](super::process_tool_calls_async)）——任务表的写方
/// 不在工具里：`local/todo_write` 是插件，够不到会话的 v2 事实源，而本函数所在的
/// 分发点是模型工具调用的**唯一入口**。这里看得见什么，事实源才记得了什么；
/// 记的必须是**观察到的声明**，不是复述。
///
/// 只记声明、不记清单的当前全量：`local/todo_write` 那份清单是纯内存的
/// （见其模块头），事实源要的是同一份状态的持久化——若在此再算一遍全量，
/// 两处各算一次就是两套真相。因此**声明里没提到的任务一律不动**
/// （`merge = true` 的增量语义，以及 `merge = false` 时被整体替换移出清单的项：
/// 后者在事实源里保持未终态——append-only 不替模型编一条「已验收」的假事实）。
pub struct TaskDeclaration {
    /// 本批声明的任务项，顺序与模型给的一致。
    pub items: Vec<TaskItem>,
}

/// 一项任务的声明态（一次 `todo_write` 调用里的一项）。
#[derive(Debug, Clone)]
pub struct TaskItem {
    /// 任务 id。**缺席时按 `task_{i+1}` 兜底**，与 `local/todo_write` 的默认同一条
    /// 格式——两处各写一次是跨插件契约（E-009 禁止 session 直引 local，所以没有
    /// 共享函数可用）：改名必须两处一起改，否则事实源的 id 会与工具清单对不上，
    /// 调度段就会念出模型不认识的 id。
    pub id: String,
    /// 任务内容（入 `task.opened` 的 `goal`）。
    pub goal: String,
    /// 依赖的任务 id。任务图是**数据不是机制**（`docs/plan/roadmap/S03` §2），
    /// 声明在清单里，成环 / 悬空由读侧 `acyclic_deps` 报。
    pub depends_on: Vec<String>,
    /// 声明状态。
    pub status: TaskStatus,
}

/// 任务状态（与 `local/todo_write` 的 `status` 枚举同集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskStatus {
    /// 未开始。
    #[default]
    Pending,
    /// 进行中。
    InProgress,
    /// 已完成（`task.asserted` 的触发态）。
    Completed,
}

/// 任务表出参的**唯一写入点**（见 [`TaskDeclaration`]）：只认 `todo_write`。
///
/// 判据取**短名**（最后一个 `/` 之后的部分）：LLM 可见全名是 `<挂载点>/<短名>`
/// （`local/todo_write`），而 `CapabilityMeta.name` 是短名——与上下文治理层的
/// 骨架化匹配（`context::window::apply_layered_sliding_window`）同一条口径，
/// 不反演字符串、不硬编码前缀。
///
/// `success = false` 不记：失败的调用没有改变清单，记它等于替工具编一条事实
/// （`note_delegation` 反过来——没履约同样要记，因为「约」本身已经存在；任务不同，
/// 清单没被改就是没被改）。
pub fn note_tasks(out: &mut Vec<TaskDeclaration>, name: &str, args: &Value, success: bool) {
    if !success {
        return;
    }
    if name.trim().rsplit('/').next().unwrap_or_default() != "todo_write" {
        return;
    }
    let Some(todos) = args.get("todos").and_then(Value::as_array) else {
        return;
    };
    let items = todos
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let goal = t.get("content").and_then(Value::as_str).unwrap_or("");
            if goal.is_empty() {
                return None;
            }
            Some(TaskItem {
                id: t
                    .get("id")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| format!("task_{}", i + 1)),
                goal: goal.to_string(),
                depends_on: t
                    .get("depends_on")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                status: match t.get("status").and_then(Value::as_str) {
                    Some("in_progress") => TaskStatus::InProgress,
                    Some("completed") => TaskStatus::Completed,
                    _ => TaskStatus::Pending,
                },
            })
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return;
    }
    out.push(TaskDeclaration { items });
}
