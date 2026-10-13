//! 控制面主体：抢占判定（[roadmap/S07](../../../../docs/stage/ST-007-插话与实时打断.md "ST-007-插话与实时打断")）与外部执行闸门（[roadmap/S09](../../../../docs/stage/ST-009-外部执行与熔断.md "ST-009-外部执行与熔断")）。

use crate::symbio_core::authz::PRINCIPAL_REFLEX;
use crate::symbio_core::Event;

/// 抢占判定结论（反射档三选一 + 超时默认，[plan/04 §2.1](../../../../docs/plan/PL-004-工程落地.md "PL-004-工程落地")）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preemption {
    /// 当前无在跑任务——插话放行，直接开始新 turn。
    Proceed,
    /// 挂起当前任务：`as_of_seq` 是挂起锚 T（恢复 = 以 T 重放投影，零状态迁移）。
    Suspend { task_id: String, as_of_seq: u64 },
    /// 排队：任务不可中断 / final 已发出（已发出的发言不可撤回）。
    Queue,
    /// 判定超时：**默认「继续」不挂起**——挂起会留下需要恢复的状态，继续不会；
    /// 两个选择都可能错，错的代价不对称，默认选代价小的（04 §2.1）。
    TimeoutDefaultContinue,
}

/// 抢占判定者（S8 第 19 步，[roadmap/S07 §3](../../../../docs/stage/ST-007-插话与实时打断.md "ST-007-插话与实时打断")）。
///
/// `pattern = decider`、`capabilities = [JudgeIntent]`、`budget_ms = 80`——
/// 系统第一次需要在几十毫秒内对外部信号做决策。**判定者只产控制事件**
/// （`task.controlled`），无 `reply.*` 写权——它不得直接发言（S07 §5，由
/// grants 表保证：`authz::ROWS` 里 [`PRINCIPAL_REFLEX`] 那一行，判据见
/// `authz.test.rs::the_reflex_row_is_narrower_than_derivation`）。
///
/// 能力那一半**不在这里**：`capabilities = [judge.intent]` 的权威是授权表，
/// 本类型只负责按 [`PRINCIPAL_REFLEX`] 这个主体名写事件。两处各写一份能力清单
/// 必然漂移成「表授予 A、闸门判 B」——两边都是合法能力名。
pub struct PreemptionDecider;

impl PreemptionDecider {
    /// 对当前在跑任务的处置判定。
    ///
    /// `elapsed_ms`：判定者自身耗时（调用方 `Instant` 计时后传入——判定是纯函数，
    /// 计时留在边界上）。预算内（≤ `budget_ms`）才做实质判定，超时走默认分支。
    /// 判定顺序（04 §2.1）：无在跑任务 → 放行；final 已发出 → 排队；否则 → 挂起。
    pub fn decide(&self, events: &[Event], elapsed_ms: u64, budget_ms: u64) -> Preemption {
        if elapsed_ms > budget_ms {
            return Preemption::TimeoutDefaultContinue;
        }
        // 在跑任务 = 最新一个 opened 且未终态、未挂起的任务。
        let mut suspended: Vec<&str> = Vec::new();
        let mut done: Vec<&str> = Vec::new();
        let mut running: Option<(&str, u64)> = None;
        for e in events {
            let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
                continue;
            };
            if e.entity == crate::symbio_core::event::Entity::Task {
                match e.kind.as_str() {
                    crate::symbio_core::event::EVENT_TASK_OPENED => {
                        running = Some((id, e.seq.map(|s| s.value()).unwrap_or(u64::MAX)));
                    }
                    crate::symbio_core::event::EVENT_TASK_ASSERTED => {
                        done.push(id);
                        suspended.retain(|x| *x != id);
                    }
                    crate::symbio_core::event::EVENT_TASK_HELD => suspended.push(id),
                    crate::symbio_core::event::EVENT_TASK_PROGRESS => {
                        suspended.retain(|x| *x != id);
                    }
                    _ => {}
                }
            }
        }
        let Some((task_id, opened_seq)) =
            running.take_if(|(id, _)| !done.contains(id) && !suspended.contains(id))
        else {
            return Preemption::Proceed;
        };
        // final 已发出（任务开启后有过收束发言）→ 排队：已发出的发言不可撤回。
        let final_after_task = events.iter().any(|e| {
            e.kind == crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                && e.seq.map(|s| s.value()).unwrap_or(u64::MAX) > opened_seq
        });
        if final_after_task {
            return Preemption::Queue;
        }
        Preemption::Suspend {
            task_id: task_id.to_string(),
            as_of_seq: opened_seq,
        }
    }

    /// 挂起事件（`task × held`）——挂起就是一条事件，不需要新状态机。
    ///
    /// 两个锚各司其职，别混：
    /// - `source_seq` = **挂起前的 head**（04 §2.2 步 1「记录当前 seq 为 T」）——
    ///   它同时是幂等键的后半段，因此必须**每次挂起都不同**：同一个任务可以被挂起
    ///   多次（挂起 → 恢复 → 再挂起），拿任务开格 seq 当锚会让第二次撞 `Duplicate`
    ///   而**静默丢掉**那次挂起（J3）。
    /// - `as_of_seq` = 任务开格 seq（[`Self::decide`] 给出的 as-of 重放锚，04 §2.2
    ///   步 5）——它是判据的一部分，随事实一起落盘，不留在内存里。
    pub fn held_event(&self, task_id: &str, source_seq: u64, as_of_seq: u64) -> Event {
        Event::pending(
            format!("held-{task_id}-{source_seq}"),
            crate::symbio_core::event::EVENT_TASK_HELD,
            crate::symbio_core::event::Entity::Task,
            crate::symbio_core::event::Verb::Held,
            0,
            PRINCIPAL_REFLEX,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "task_id": task_id, "as_of": as_of_seq }))
    }

    /// 恢复事件（`task × progressed`）——挂起的逆操作，同样是一条事件。
    ///
    /// 与 [`Self::held_event`] 成对：**只挂不收就是永久挂起**（挂起期间任务不进
    /// `readyset`，而 `readyset` 又是模型唯一的调度候选来源 ⇒ 任务从此再也不被
    /// 调度，且没有任何一条消息说得出为什么）。故插话轮一结束就由调用方写本条。
    /// 锚同样取写入时刻的 head：同一任务的第二次挂起要有第二次恢复。
    pub fn resume_event(&self, task_id: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("res-{task_id}-{source_seq}"),
            crate::symbio_core::event::EVENT_TASK_PROGRESS,
            crate::symbio_core::event::Entity::Task,
            crate::symbio_core::event::Verb::Progressed,
            0,
            PRINCIPAL_REFLEX,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "task_id": task_id }))
    }

    /// 控制事件（`control × opened`）——打断处置的产出事实。
    pub fn control_event(&self, reason: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("ctrl-{source_seq}"),
            crate::symbio_core::event::EVENT_CONTROL_OPENED,
            crate::symbio_core::event::Entity::Control,
            crate::symbio_core::event::Verb::Opened,
            0,
            PRINCIPAL_REFLEX,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "reason": reason }))
    }
}

/// 闸门结论（S8 第 20 步，[roadmap/S09 §6](../../../../docs/stage/ST-009-外部执行与熔断.md "ST-009-外部执行与熔断")）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// 未授权——调用方**不得产生任何事件**（验收 1：拒绝且无事件）。
    Refuse,
    /// 熔断（预算耗尽 / 判定超时）——调用方**必须**写 `task.controlled` 事件
    /// （载荷 `reason`），不允许静默继续（验收 2）。
    Break { reason: &'static str },
    /// 放行执行。
    Allow,
}

/// 熔断闸门（S8 第 20 步，[roadmap/S09](../../../../docs/stage/ST-009-外部执行与熔断.md "ST-009-外部执行与熔断")）。
///
/// `budget_ms = 80` 的反射档判定：超预算 / 超时**熔断**而不是"先做了再说"。
/// 授权判定留在调用方（读路径持矩阵）——本体的输入是判据数据，不是 ⑥ 的句柄。
pub struct CircuitBreaker;

impl CircuitBreaker {
    /// 外部执行闸门。
    ///
    /// - `authorized`：主体是否持外部执行能力（`ProduceArtifact`，读路径判定）；
    /// - `spent_ms` / `requested_ms` / `budget_ms`：已耗 / 本次申请 / 总预算；
    /// - `elapsed_ms`：闸门自身耗时——超反射档预算也熔断（**有事件**的超时，
    ///   不是静默失效；I3）。
    pub fn gate(
        &self,
        authorized: bool,
        spent_ms: u64,
        requested_ms: u64,
        budget_ms: u64,
        elapsed_ms: u64,
    ) -> GateDecision {
        if elapsed_ms > crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms() {
            return GateDecision::Break {
                reason: "gate-timeout",
            };
        }
        if !authorized {
            return GateDecision::Refuse;
        }
        if spent_ms.saturating_add(requested_ms) > budget_ms {
            return GateDecision::Break {
                reason: "budget-exhausted",
            };
        }
        GateDecision::Allow
    }

    /// 熔断事件（复用 `control/opened` 格，载荷 `reason` 区分打断与熔断）。
    pub fn break_event(&self, reason: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("cb-{source_seq}"),
            crate::symbio_core::event::EVENT_CONTROL_OPENED,
            crate::symbio_core::event::Entity::Control,
            crate::symbio_core::event::Verb::Opened,
            0,
            PRINCIPAL_REFLEX,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "reason": reason }))
    }
}
