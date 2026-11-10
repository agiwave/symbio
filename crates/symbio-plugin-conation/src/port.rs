//! `conation` 端口的执行面：把一条「欲」升格成 `task.opened`（DE-042 / `D-385` / `D-386`）。
//!
//! ## 谁判、谁写
//!
//! 欠不欠在 [`crate::debt`] 判（开关 / 授权 / 已声明）；本文件是那一笔债的**动作点**：
//! 执行体自判授权并交 [`AuthzVerdict`]（`D-385` 的「环不判、能力自判」），通过后把
//! `task.opened` 作为世界事实草稿交给宿主的唯一写口。
//!
//! ## 编译期唯一出口
//!
//! [`ApprovedIntent`] 字段私有，模块外写不出结构体字面量；唯一构造点是
//! [`IntentGate::approve`]，它先判授权。执行体把 `ApprovedIntent` 变成 `task.opened`
//! 草稿——绕过闸门就没有可升格的意图（`DE-001` E2「欲不得直接变成行」）。
//!
//! ## 溯源与命名
//!
//! - `produced_by` 指回**那条欲事件的 seq**（`conation.intents` 的升格靠它认，不靠
//!   goal 字符串相等）；seq 从读面按 `intent_id` 回读，不自己计数。
//! - `task_id = hb-{触发 seq}` 与草稿键 `o-hb-{触发 seq}` 保持迁移前的形状：同一个
//!   触发重放时幂等键相同，事实源吸收重复投递。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::events::{EVENT_CONATION_EXPRESSED, EVENT_TASK_OPENED};
use symbio_plugin_api::port::{
    PortCapabilities, PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome,
    PortProvider,
};
use symbio_plugin_api::{Entity, LatencyTier, Verb};

use crate::debt::PROMOTE_PORT;

/// 授权拒绝的固定理由（与迁移前 `IntentGate` 的措辞同一个来源）。
pub(crate) const UNAUTHORIZED: &str = "unauthorized: define.work";

pub(crate) struct ConationPort;

impl PortProvider for ConationPort {
    fn ports(&self) -> Vec<String> {
        vec![PROMOTE_PORT.to_string()]
    }

    fn provider_id(&self) -> &str {
        PROMOTE_PORT
    }

    fn capabilities(&self) -> PortCapabilities {
        PortCapabilities {
            idempotent: true,
            ..PortCapabilities::default()
        }
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(PromoteExecutor))
    }
}

struct PromoteExecutor;

#[async_trait]
impl PortExecutor for PromoteExecutor {
    async fn execute(&self, ctx: PortExecContext, input: PortInput) -> PortOutcome {
        // 执行体自判（`D-385`）：不表态在类型上不可表达——`Denied` 总要带理由。
        if !crate::debt::authorized() {
            return PortOutcome::denied(UNAUTHORIZED);
        }
        let intent_id = str_of(&input.payload, "intent_id");
        let goal = str_of(&input.payload, "goal");
        let Some(triggered_seq) = input.payload.get("source_seq").and_then(Value::as_u64) else {
            return PortOutcome::failed(PortErrorClass::NonRetryable, "债载荷缺 source_seq");
        };
        let Some(from_seq) = expressed_seq(&ctx, &intent_id) else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                format!("欲事件回读不到或没有溯源：{intent_id}"),
            );
        };
        let approved = match IntentGate::approve(&goal, from_seq) {
            Ok(approved) => approved,
            Err(reason) => return PortOutcome::denied(reason),
        };
        ctx.emit
            .emit(vec![open_task(&approved, triggered_seq)])
            .await;
        PortOutcome::ok()
    }
}

/// 经闸门批准后的可执行意图（**只有** [`IntentGate::approve`] 能造出来：字段私有，
/// 模块外连结构体字面量都写不出——这是 E2「欲不得直接变成行」的编译期兑现）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovedIntent {
    from_seq: u64,
    goal: String,
}

impl ApprovedIntent {
    /// 溯源锚：升格自哪一条 `conation.expressed`。
    pub(crate) fn source_seq(&self) -> u64 {
        self.from_seq
    }

    /// 被批准的目标文本。
    pub(crate) fn goal(&self) -> &str {
        &self.goal
    }
}

/// 升格闸：授权**不是**一句声明，判据取 `production_matrix` 的那一份（`define.work`）。
struct IntentGate;

impl IntentGate {
    /// 升格：唯一能把目标文本变成 [`ApprovedIntent`] 的函数。
    fn approve(goal: &str, from_seq: u64) -> Result<ApprovedIntent, String> {
        if !crate::debt::authorized() {
            return Err(UNAUTHORIZED.to_string());
        }
        Ok(ApprovedIntent {
            from_seq,
            goal: goal.to_string(),
        })
    }
}

/// 那条欲事件的 `seq`：按事实身份回读，且**必须带溯源**（I2 强化——来路不明的意图
/// 混不进来）。
fn expressed_seq(ctx: &PortExecContext, intent_id: &str) -> Option<u64> {
    let event = ctx.reader.as_ref()?.events().into_iter().find(|e| {
        e.event_id == intent_id && e.kind == EVENT_CONATION_EXPRESSED && e.produced_by.is_some()
    })?;
    event.seq.map(|s| s.value())
}

/// 升格产物：长目标任务（自主层 `budget_ms` 的完整取值）。
fn open_task(approved: &ApprovedIntent, triggered_seq: u64) -> FactDraft {
    let task_id = format!("hb-{triggered_seq}");
    FactDraft {
        key: format!("o-{task_id}"),
        kind: EVENT_TASK_OPENED.to_string(),
        entity: Entity::Task,
        verb: Verb::Opened,
        payload: json!({
            "task_id": task_id,
            "depends_on": [],
            "goal": approved.goal(),
            "budget_ms": LatencyTier::Autonomic.budget_ms(),
        }),
        produced_by: Some(approved.source_seq()),
    }
}

fn str_of(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
