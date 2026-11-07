//! 工具债：只问「有没有 `tool.requested` 这条事实」，**是否欠**不按载荷分叉。
//!
//! 模型执行体在一次带工具调用的往返里写**一条** `tool.requested`（载荷 = 这次往返请求的调用
//! 清单）；本行从这条事实的身份派生债 id（禁计数），并把**触发事实 id**放进载荷——
//! 执行体据此精确配对那一条 `tool.requested`，不靠任何编号。债表永远只问存在性，
//! 按内容分叉的责任留在写方。
//!
//! **配对例外（D-369）**：恢复行必须读载荷里的 wire id 才找得到「这条答复属于哪条请求」。
//! `user.answer.call_id` 与 `tool.requested.calls[].id` 是同一个 provider wire id；
//! 按它反查，而不是拿「最近一条请求」顶替——两条请求同时在场时，最近的那条不是配对。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{EVENT_TOOL_REQUESTED, EVENT_USER_ANSWER};
use symbio_plugin_api::CapabilityRow;

use super::port::TOOL;

/// 一行工具债。
pub(crate) struct ToolDebts;

impl DebtSource for ToolDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let mut rows = Vec::new();
        // 一笔 `tool.requested` 欠一次执行：首派（无 `sent`）由环直接派发。
        // 触发事实是**旧**的 `tool.requested`，挂起之后它不再满足「新触发」。
        if let Some(request_id) = facts.latest_fact_id(EVENT_TOOL_REQUESTED) {
            rows.push(
                DebtSpec::new(
                    "tool-calls",
                    TOOL,
                    format!("tool:{request_id}"),
                    json!({ "request_id": request_id }),
                )
                .when(EVENT_TOOL_REQUESTED)
                .lease(600_000, 2),
            );
        }
        // 外部答复：一条 `user.answer` 欠一次「把答复落成那次调用的结果」。
        // 债表只问存在性、不按载荷分叉；**配对**读 wire id：答复自带的 `call_id` 反查
        // 它所属的那条 `tool.requested`（见模块文档 D-369），不拿「最近一条请求」顶替。
        // 改回原 key（与首派行同一个 `tool:<request_id>`），收束因此落在同一
        // `(port, id, attempt)` 上——`attempt` 不加、不开新债、不新 `request`。
        // 同一请求的多条答复只认最后一条：一次恢复只消费一条答复；批内多卡需要
        // 逐调用债务（批债 / 逐笔债另议，见任务台账的「债行同族写法」一节）。
        let mut resume: Vec<(String, String)> = Vec::new();
        for (answer_id, payload) in facts.facts_of(EVENT_USER_ANSWER) {
            let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(request_id) = request_owning_call(facts, call_id) else {
                continue;
            };
            match resume.iter_mut().find(|(known, _)| known == &request_id) {
                Some((_, slot)) => *slot = answer_id.to_string(),
                None => resume.push((request_id, answer_id.to_string())),
            }
        }
        for (request_id, answer_id) in resume {
            rows.push(
                DebtSpec::new(
                    "tool-resume",
                    TOOL,
                    format!("tool:{request_id}"),
                    json!({ "resume": "answer", "answer_event": answer_id }),
                )
                .when(EVENT_USER_ANSWER),
            );
        }
        rows
    }
}

/// 这次调用属于哪条 `tool.requested`：按答复自带的 `call_id` 在请求载荷的
/// `calls[].id` 里反查（两者都是 provider 的 wire id）。
///
/// 找不到 ⇒ `None`：那条请求可能已被压缩 / 归档。**不拿别的请求顶替**——顶替会把
/// 这条答复的调用结果落进另一轮的工具结果里，账上却记在另一笔债上。
fn request_owning_call(facts: &DebtFacts, call_id: &str) -> Option<String> {
    facts
        .facts_of(EVENT_TOOL_REQUESTED)
        .into_iter()
        .rev()
        .find(|(_, payload)| {
            payload
                .get("calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| {
                    calls
                        .iter()
                        .any(|call| call.get("id").and_then(Value::as_str) == Some(call_id))
                })
        })
        .map(|(id, _)| id.to_string())
}

/// 本插件申报的债行（`debt` 族的登记）。
pub(crate) fn debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row("tool", Arc::new(ToolDebts))]
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;
