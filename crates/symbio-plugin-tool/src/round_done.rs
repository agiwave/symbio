//! `tool.calls.done` 的簿记写方（D-378）：收尾是**投影**，不是执行体的最后一步。
//!
//! 执行体只负责一笔调用；「这批调用齐了」不写在任何进程的收尾语句里——宿主每次驱动
//! 前后都把全量投影交给这里，本写方按 `tool.requested.calls[]` 与各笔债的终局事实
//! 重算：**每一笔的最大 attempt 都有收束** ⇒ 这一请求完成，补写
//! `tool.calls.done:{request_id}`。
//!
//! 于是执行体中途死掉也不会有「永久未完成」：那一笔由租约收束（重试或判死），收束
//! 落进同一投影后，本写方下一圈就把它算进去。幂等由事实键承担：同一 `request_id`
//! 只长出一条完成事实，事实源按 `event_id` 吸收重复。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_plugin_api::drive::DriveFact;
use symbio_plugin_api::events::EVENT_TOOL_REQUESTED;
use symbio_plugin_api::termination::{termination_row, TerminationView, TerminationWriter};
use symbio_plugin_api::{CapabilityRow, Entity, FactDraft, Verb};

use super::port::TOOL;

/// 单消费方事实：这次往返的工具清单已还完（会话推进行按它开账）。
pub(crate) const TOOL_CALLS_DONE: &str = "tool.calls.done";

/// 工具轮的完成写方。
pub(crate) struct ToolCallsDone;

impl TerminationWriter for ToolCallsDone {
    fn observe(&self, view: &TerminationView<'_>) -> Vec<FactDraft> {
        let mut out = Vec::new();
        for request in view
            .facts
            .iter()
            .filter(|fact| fact.kind == EVENT_TOOL_REQUESTED)
        {
            let request_id = request.key.id.as_str();
            if done_written(view.facts, request_id) {
                continue;
            }
            let Some(calls) = request.payload.get("calls").and_then(Value::as_array) else {
                continue;
            };
            if calls.is_empty() {
                continue;
            }
            let mut count = 0usize;
            let mut complete = true;
            for call in calls {
                let Some(call_id) = call.get("id").and_then(Value::as_str) else {
                    complete = false;
                    break;
                };
                count += 1;
                if !terminal(view.facts, request_id, call_id) {
                    complete = false;
                    break;
                }
            }
            if !complete {
                continue;
            }
            out.push(FactDraft {
                key: format!("{TOOL_CALLS_DONE}:{request_id}"),
                kind: TOOL_CALLS_DONE.to_string(),
                entity: Entity::Scope,
                verb: Verb::Asserted,
                payload: json!({ "requested": request_id, "calls": count }),
                produced_by: None,
            });
        }
        out
    }
}

/// 这一请求的完成事实是否已经落过（键即幂等）。
fn done_written(facts: &[DriveFact], request_id: &str) -> bool {
    let key = format!("{TOOL_CALLS_DONE}:{request_id}");
    facts
        .iter()
        .any(|fact| fact.kind == TOOL_CALLS_DONE && fact.key.id == key)
}

/// 这笔调用的**当前** attempt 是否已有收束。
///
/// 只看最大 attempt：重试后旧 attempt 的迟归即便落在投影里，也不能冒充这一笔完成。
fn terminal(facts: &[DriveFact], request_id: &str, call_id: &str) -> bool {
    let debt_id = format!("tool:{request_id}:{call_id}");
    let mut current = 0u32;
    for fact in facts
        .iter()
        .filter(|fact| fact.key.port == TOOL && fact.key.id == debt_id)
    {
        current = current.max(fact.key.attempt);
    }
    current > 0
        && facts.iter().any(|fact| {
            fact.key.port == TOOL
                && fact.key.id == debt_id
                && fact.key.attempt == current
                && fact.closing().is_some()
        })
}

/// 本插件申报的簿记写方行。
pub(crate) fn rows() -> Vec<CapabilityRow> {
    vec![termination_row("tool-calls-done", Arc::new(ToolCallsDone))]
}

#[cfg(test)]
#[path = "round_done.test.rs"]
mod tests;
