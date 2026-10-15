//! 自主发起者（[roadmap/S12](../../../../docs/003-stage/012-自主层与长期目标.md "ST-012-自主层与长期目标")）：定时触发、表达欲、开长目标任务、健康自检。

use crate::symbio_core::authz::PRINCIPAL_AUTONOMOUS;
use crate::symbio_core::Event;

/// 自主发起者（S9 第 21 步，[roadmap/S12](../../../../docs/003-stage/012-自主层与长期目标.md "ST-012-自主层与长期目标")）。
///
/// 形状 = `pattern: decider`（规则驱动，零 LLM）、能力只 `define.work`、
/// `budget_ms` = 自主档 [`LatencyTier::Autonomic`]（四层时延的第四个取值——自主层不是
/// 新架构层，只是同一个参数的第四个取值）。
///
/// ## 两处「单点定义」，本类型各引用一次
///
/// - **主体名**：[`PRINCIPAL_AUTONOMOUS`]——表里那一行与这里四个事件的 `actor` 是同一个
///   常量。名字分两处写，授权表那行就成了一条谁也管不到的死记录（见该常量的文档）。
/// - **预算**：[`LatencyTier::Autonomic`]——`86400000` 不在本文件出现第二次。
///
/// 能力那一半**不在这里**：`capabilities = [define.work]` 的权威是授权表
/// （`authz::ROWS` 里 [`PRINCIPAL_AUTONOMOUS`] 那一行），本类型只负责按那个主体名写事件。
/// 两处各写一份能力清单必然漂移成「表授予 A、闸门判 B」——两边都是合法能力名。
pub struct AutonomousInitiator;

impl AutonomousInitiator {
    /// 定时触发：**触发器产出事件，不是旁路**——自主行为同样走 I1 单通道。
    pub fn trigger(&self, source_seq: u64) -> Event {
        Event::pending(
            format!("sys-{source_seq}"),
            crate::symbio_core::event::EVENT_SYSTEM_TRIGGERED,
            crate::symbio_core::event::Entity::System,
            crate::symbio_core::event::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "kind": "scheduled" }))
    }

    /// 表达一条「欲」（E1：欲是数据，必带溯源——否则过不了 IntentGate）。
    pub fn express_intent(&self, goal: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("want-{source_seq}"),
            crate::symbio_core::event::EVENT_CONATION_EXPRESSED,
            crate::symbio_core::event::Entity::Conation,
            crate::symbio_core::event::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "goal": goal }))
    }

    /// 把闸门批准的意图落成长目标任务（自主层 `budget_ms` 的完整取值）。
    pub fn open_long_goal(&self, task_id: &str, goal: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("o-{task_id}"),
            crate::symbio_core::event::EVENT_TASK_OPENED,
            crate::symbio_core::event::Entity::Task,
            crate::symbio_core::event::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "task_id": task_id,
            "depends_on": [],
            "goal": goal,
            "budget_ms": crate::symbio_core::adapters::LatencyTier::Autonomic.budget_ms(),
        }))
    }

    /// 健康自检：触发时刻测得的状态入格（`system × progressed`）。
    ///
    /// [`Self::trigger`] 说"触发了"，本条说"触发时测得什么"——`system` 实体的两个
    /// 格子一个开一个进（S12 §2 / [plan/01 §6](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")
    /// 的 `system` 行）。缺了它，健康自检永远没有事实，而"名字是数据、单点定义"
    /// （ADR-043）又不允许插件自己拼 `"system.health"` 字面量——写入口只能在这里。
    ///
    /// `source_seq` 取触发事实的 seq：自检是**这一次触发**的观测附录，不是独立事件。
    pub fn health_event(&self, source_seq: u64, idle_ms: i64) -> Event {
        Event::pending(
            format!("health-{source_seq}"),
            crate::symbio_core::event::EVENT_SYSTEM_HEALTH,
            crate::symbio_core::event::Entity::System,
            crate::symbio_core::event::Verb::Progressed,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "idle_ms": idle_ms }))
    }
}
