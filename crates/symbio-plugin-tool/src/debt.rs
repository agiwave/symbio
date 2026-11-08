//! 工具债：**一调用一笔**（D-378）——只问「有没有 `tool.call.requested` 这条标记」。
//!
//! 模型执行体在一次带工具调用的往返里写一条 `tool.requested`（组织键）与 N 条
//! `tool.call.requested`（逐调用标记，载荷 `{request_id, call_id}`）。本行按标记
//! **逐条**生成债（行本身就承载了那条标记的身份）：键 `tool:{request_id}:{call_id}`。
//! 债表永远只问存在性，不读 `tool.requested` 的 `calls[]`——拆分在写方（I-2 不破）。
//!
//! **恢复按 `call_id` 一对一**：每条 `user.answer` 只恢复它自己那次调用；同一请求的
//! 多张卡各欠各的恢复，不再「一次恢复消费整批」。恢复行与首派行用**同一个债 id**
//! （`tool:{request_id}:{call_id}`），收束因此落在同一 `(port, id, attempt)` 上——
//! `attempt` 不加、不开新债、不新 `request`。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use symbio_plugin_api::debt::{
    debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest, FailureProjection,
};
use symbio_plugin_api::drive::DRIVE_SENT;
use symbio_plugin_api::events::{EVENT_TOOL_CALL_REQUESTED, EVENT_USER_ANSWER};
use symbio_plugin_api::CapabilityRow;

use super::port::TOOL;

/// 一行工具债。
pub(crate) struct ToolDebts;

impl DebtSource for ToolDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let mut rows = Vec::new();
        // 标记 → 一次调用一笔债。每条标记都欠；不同的 `request_id` 即使复用同一
        // wire id 也是两笔债（`call_id` 只在本请求内唯一）。
        let mut owners: Vec<(&str, &str)> = Vec::new();
        // 同一请求内按 `calls[]` 顺序串一条**派发链**：后一笔的 `when_id` 绑前一笔的
        // `sent` 事实（环的「已派发」，按债 id 读），**不是**收束事实。等收束会让一笔
        // `sent{deadline_ms:null}`（待批）堵住整条链，后面的调用永远选不到（D-378）；
        // 等派发只钉住首派次序，挂起的那笔不挡后面，重试 / 租约仍按笔计。
        //
        // 口径：读面是 `exists_id(kind, id)`，只比 `kind + key.id`，**不比 attempt**——
        // 问的是「这一 id 有过 `sent`」，不是「attempt 1 的 `sent`」。因此判定单调：
        // 前一笔重试到 attempt 2 也不会把后一笔重新挂住。
        let mut chains: HashMap<&str, Vec<String>> = HashMap::new();
        for (_, payload) in facts.facts_of(EVENT_TOOL_CALL_REQUESTED) {
            let Some(request_id) = payload.get("request_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
                continue;
            };
            owners.push((call_id, request_id));
            let debt_id = format!("tool:{request_id}:{call_id}");
            let mut spec = DebtSpec::new(
                "tool-call",
                TOOL,
                debt_id.clone(),
                json!({ "request_id": request_id, "call_id": call_id }),
            )
            .when(EVENT_TOOL_CALL_REQUESTED)
            .lease(600_000, 2)
            .failure_card(FailureProjection::ToolCall);
            if let Some(previous) = chains.get(request_id).and_then(|ids| ids.last()) {
                spec = spec.when_id(DRIVE_SENT, previous.clone());
            }
            chains.entry(request_id).or_default().push(debt_id);
            rows.push(spec);
        }
        // 外部答复：一条 `user.answer` 欠一次「把答复落成**它自己那次**调用的结果」。
        // 配对只认 wire id，不拿「最近一条请求」顶替。
        let mut resume: Vec<(&str, String)> = Vec::new();
        for (answer_id, payload) in facts.facts_of(EVENT_USER_ANSWER) {
            let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
                continue;
            };
            match resume.iter_mut().find(|(known, _)| *known == call_id) {
                Some((_, slot)) => *slot = answer_id.to_string(),
                None => resume.push((call_id, answer_id.to_string())),
            }
        }
        for (call_id, answer_id) in resume {
            // 复用同一 wire id 的多条标记：答复只带 call_id，按最后记入的那条配对
            // （与旧口径一致；无法区分时取最新的那一笔，不猜更早的）。
            let Some((_, request_id)) = owners.iter().rev().find(|(known, _)| *known == call_id)
            else {
                // 标记不在了（压缩 / 归档）⇒ 不恢复。不拿别的调用顶替：顶替会把
                // 这条答复落进另一笔债，账却记在另一处。
                continue;
            };
            let debt_id = format!("tool:{request_id}:{call_id}");
            let mut spec = DebtSpec::new(
                "tool-resume",
                TOOL,
                debt_id.clone(),
                json!({
                    "resume": "answer",
                    "answer_event": answer_id,
                    "request_id": request_id,
                    "call_id": call_id,
                }),
            )
            .when(EVENT_USER_ANSWER)
            .failure_card(FailureProjection::ToolCall);
            // 与首派同一条派发链：恢复也要等前一笔已派发（不等它收束）。
            if let Some(previous) = chains
                .get(request_id)
                .and_then(|ids| ids.iter().position(|id| id == &debt_id))
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| chains.get(request_id).and_then(|ids| ids.get(index)))
            {
                spec = spec.when_id(DRIVE_SENT, previous.clone());
            }
            rows.push(spec);
        }
        rows
    }
}

/// 本插件申报的债行（`debt` 族的登记）。
pub(crate) fn debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row("tool", Arc::new(ToolDebts))]
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;
