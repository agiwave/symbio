//! `tool.calls.done` 的簿记行（D-378 / D-508）：收尾是**投影**，不是执行体的最后一步。
//!
//! 执行体只负责一笔调用；「这批调用齐了」不写在任何进程的收尾语句里——本行按
//! `tool.requested.calls[]` 与工具端口的债状态重算：**这一请求的工具债全部收束**
//! ⇒ 补写 `tool.calls.done:{request_id}`。
//!
//! ## 为什么是债行，不是终止写方（D-508）
//!
//! 终止写方只在宿主驱动**前后**观察（D-367），而一次驱动可以跨过多个模型往返：
//! 工具轮一结束，下一轮请求可能在同一驱动里已经跑完并写下终答。那时边界事实落到
//! 终答之后，会话的进度面按 `tool.calls.done` 判定「本轮已经答完」⇒ 该边界的一句
//! 汇报被静默吞掉（t23 的并发红灯）。债行在**工具端口当圈没人认领**时就还
//! （`after_port`），比终止写方早一整拍：边界事实因此总在下一轮模型往返的计划之前
//! 落库，汇报不再与终答抢时序。
//!
//! ## 幂等
//!
//! 同一 `request_id` 只长出一条完成事实：债行声明 `unless_id(TOOL_CALLS_DONE, …)`，
//! 事实源按 `event_id` 吸收重复。

use serde_json::{json, Value};

use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::EVENT_TOOL_REQUESTED;
use symbio_plugin_api::rows::{RowHandler, RowRequest, RowStep, PLUGIN_ROW};
use symbio_plugin_api::{CapabilityRow, Entity, FactDraft, Verb, PLUGIN_ID_TOOL};

use super::port::TOOL;

/// 单消费方事实：这次往返的工具清单已还完（会话推进行按它开账）。
pub(crate) const TOOL_CALLS_DONE: &str = "tool.calls.done";

/// 债行名（也是 `RowHandler::row` 的键）。
const TOOL_CALLS_DONE_ROW: &str = "tool-calls-done";

/// 完成事实的幂等键。
fn done_key(request_id: &str) -> String {
    format!("{TOOL_CALLS_DONE}:{request_id}")
}

/// 完成行的债表：一个 `tool.requested` 欠一条「把边界事实落下来」。
///
/// `after_port(TOOL)` 是**唯一**的完成判据：工具端口当圈没人认领 ⇒ 这一请求的
/// 每一笔调用都已有收束（应答 / 判死）。逐 attempt 的终局判定由端口状态承担，
/// 本行不复制那份算术。
pub(crate) struct ToolCallsDoneDebts;

impl DebtSource for ToolCallsDoneDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let mut out = Vec::new();
        for (request_id, payload) in facts.facts_of(EVENT_TOOL_REQUESTED) {
            let Some(calls) = payload.get("calls").and_then(Value::as_array) else {
                continue;
            };
            if calls.is_empty() {
                continue;
            }
            let done_id = done_key(request_id);
            if facts.fact_payload(TOOL_CALLS_DONE, &done_id).is_some() {
                continue;
            }
            out.push(
                DebtSpec::new(
                    TOOL_CALLS_DONE_ROW,
                    PLUGIN_ROW,
                    done_id.clone(),
                    json!({
                        "row": TOOL_CALLS_DONE_ROW,
                        "key": [
                            ["request", request_id],
                            ["calls", calls.len().to_string()],
                        ],
                    }),
                )
                .when_id(EVENT_TOOL_REQUESTED, request_id)
                .unless_id(TOOL_CALLS_DONE, done_id)
                .after_port(TOOL)
                .lease(60_000, 2),
            );
        }
        out
    }
}

/// 完成行：把「这一请求已齐」落成事实。跑到本行时端口已干净（见 [`ToolCallsDoneDebts`]），
/// 因此不需要再读一遍各笔调用的终局。
pub(crate) struct ToolCallsDoneRow;

impl RowHandler for ToolCallsDoneRow {
    fn owner(&self) -> &'static str {
        PLUGIN_ID_TOOL
    }

    fn row(&self) -> &'static str {
        TOOL_CALLS_DONE_ROW
    }

    fn writes(&self) -> &'static [&'static str] {
        &[TOOL_CALLS_DONE]
    }

    fn run(&self, req: &RowRequest<'_>) -> RowStep {
        let Some(request_id) = req.key_of("request") else {
            return RowStep::Facts(Vec::new());
        };
        let count = req
            .key_of("calls")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        if count == 0 {
            return RowStep::Facts(Vec::new());
        }
        RowStep::Facts(vec![FactDraft {
            key: done_key(request_id),
            kind: TOOL_CALLS_DONE.to_string(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: json!({ "requested": request_id, "calls": count }),
            produced_by: Some(req.user_seq),
        }])
    }
}

/// 本插件申报的完成行债表。
pub(crate) fn debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row(
        TOOL_CALLS_DONE_ROW,
        std::sync::Arc::new(ToolCallsDoneDebts),
    )]
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按行名选它。
symbio_plugin_api::submit_row!(ToolCallsDoneRow);

#[cfg(test)]
#[path = "round_done.test.rs"]
mod tests;
