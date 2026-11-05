//! 工具债：只问「有没有 `tool.requested` 这条事实」，不读它的载荷。
//!
//! 模型执行体在一次带工具调用的往返里写**一条** `tool.requested`（载荷 = 这次往返请求的调用
//! 清单）；本行从这条事实的身份派生债 id（禁计数），并把**触发事实 id**放进载荷——
//! 执行体据此精确配对那一条 `tool.requested`，不靠任何编号。债表永远只问存在性，
//! 按内容分叉的责任留在写方。

use std::sync::Arc;

use serde_json::json;
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::EVENT_TOOL_REQUESTED;
use symbio_plugin_api::CapabilityRow;

use super::port::TOOL;

/// 一行工具债。
pub(crate) struct ToolDebts;

impl DebtSource for ToolDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(request_id) = facts.latest_fact_id(EVENT_TOOL_REQUESTED) else {
            return Vec::new();
        };
        vec![DebtSpec::new(
            "tool-calls",
            TOOL,
            format!("tool:{request_id}"),
            json!({ "request_id": request_id }),
        )
        .when(EVENT_TOOL_REQUESTED)
        .lease(600_000, 2)]
    }
}

/// 本插件申报的债行（`debt` 族的登记）。
pub(crate) fn debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row("tool", Arc::new(ToolDebts))]
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;
