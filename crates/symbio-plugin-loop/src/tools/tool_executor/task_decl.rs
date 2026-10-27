//! 任务表声明出参的**记账口**（一次**成功**的 `todo_write` 声明的清单状态）：观察点是
//! 模型工具调用的唯一入口，声明随轮末收束落成 `task.*` 格。
//!
//! 记录类型 [`TaskDeclaration`] / [`TaskItem`] / [`TaskStatus`] 不住这里——它们隔着
//! 驱动器与留守存储两侧都要用，住在 [`symbio_core::facts`]
//! （[D-082](../../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三 5 组）。本文件只留
//! **判据与门**：哪一次调用算声明、声明里有什么——那是机制，随观察点住插件侧。

use serde_json::Value;
use symbio_core::facts::{TaskDeclaration, TaskItem, TaskStatus};

/// 任务表出参的**唯一写入点**（见 [`TaskDeclaration`]）：只认**声明了
/// `task_table` 行为的工具**（`local/todo_write` 即是； 改了判据的住址，
/// 没改这条语义）。
///
/// 匹配口径取**短名**（最后一个 `/` 之后的部分）：LLM 可见全名是 `<挂载点>/<短名>`
/// （`local/todo_write`），而 `CapabilityMeta.name` 是短名——不反演字符串、不硬编码前缀。
///
/// `success = false` 不记：失败的调用没有改变清单，记它等于替工具编一条事实
/// （承诺的效果反过来——没履约同样要记，因为「约」本身已经存在；任务不同，
/// 清单没被改就是没被改）。
pub fn note_tasks(
    out: &mut Vec<TaskDeclaration>,
    decls: &super::ToolDecls,
    name: &str,
    args: &Value,
    success: bool,
) {
    if !success {
        return;
    }
    if !decls.owns_task_table(name) {
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
