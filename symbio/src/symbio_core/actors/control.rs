//! 控制面主体：抢占判定（[roadmap/S07](../../../../docs/007-task/127-插话与实时打断.md ")）、外部执行闸门（[roadmap/S09](../../../../docs/007-task/129-外部执行与熔断.md ")）与淘汰回路（[roadmap/S13](../../../../docs/007-task/133-多智能体社会.md ")）。

use crate::symbio_core::authz::{PRINCIPAL_ARBITER, PRINCIPAL_REFLEX};
use crate::symbio_core::governance::GrantsChange;
use crate::symbio_core::projection::reputation::ReputationEntry;
use crate::symbio_core::{ActorSpec, Event, Pattern, Scope};

/// 抢占判定结论（反射档三选一 + 超时默认，[plan/04 §2.1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
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

/// 抢占判定者（S8 第 19 步，[roadmap/S07 §3](../../../../docs/007-task/127-插话与实时打断.md ")）。
///
/// `pattern = decider`、`capabilities = [JudgeIntent]`、`budget_ms = 80`——
/// 系统第一次需要在几十毫秒内对外部信号做决策。**判定者只产控制事件**
/// （`task.controlled`），无 `reply.*` 写权——它不得直接发言（S07 §5，由
/// grants 表保证：`authz::ROWS` 里 [`PRINCIPAL_REFLEX`] 那一行，判据见
/// `authz.test.rs::the_reflex_row_is_narrower_than_main`）。
///
/// 能力那一半**不在这里硬写**：`capabilities = [judge.intent]` 的权威是授权表，
/// [`Self::spec`] 从表行取（`production_matrix` ⇒ `grant_names_of`）。两处各写一份能力清单
/// 必然漂移成「表授予 A、闸门判 B」——两边都是合法能力名。
pub struct PreemptionDecider;

impl PreemptionDecider {
    /// S07 抢占判定者**行**（`ActorSpec` 生产构造点，相应批次 批②）。
    ///
    /// `{ principal: session/reflex, pattern: decider, capabilities: [judge.intent],
    /// budget_ms: 80, scope: root }`——五个字段全部**取实**，与 S07 §3 表逐字相符：
    /// principal/pattern/capabilities 同源于授权表那一行（相应批次 落地的
    /// `PRINCIPAL_REFLEX` 窄行），budget = 反射档，scope = root。
    ///
    /// 判定者必须有**行**：若 `decide` 被直驱、预算由调用方现算、规格从不构造，就是
    /// 「判定真、行假」。本构造点让预算与能力有了随主体走的载体，读方见
    /// [`Self::decide`]（预算）与 `plugins::session::transcript::inbox` 的写侧闸
    /// （能力，[`ActorSpec::can_judge_intent`]）。
    ///
    /// 相应批次: ../../../../docs/007-task/081-授权表缺行登记批.md "
    pub fn spec(&self) -> ActorSpec {
        let principal = PRINCIPAL_REFLEX.to_string();
        ActorSpec {
            pattern: Pattern::for_principal(&principal),
            capabilities: crate::symbio_core::authz::production_matrix().grant_names_of(&principal),
            budget_ms: crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms(),
            scope: Scope::Root,
            principal,
        }
    }

    /// 对当前在跑任务的处置判定。
    ///
    /// `spec`：判定者**自己的行**（[`Self::spec`]）。预算从它身上读
    /// （`spec.budget_ms`， 批②）——不再是调用方现算一个裸 `u64` 塞进来，
    /// 「判定者行声明了多少预算」因此是一等数据、有真读方。
    ///
    /// `elapsed_ms`：判定者自身耗时（调用方 `Instant` 计时后传入——判定是纯函数，
    /// 计时留在边界上）。预算内（≤ `budget_ms`）才做实质判定，超时走默认分支。
    /// 判定顺序（04 §2.1）：无在跑任务 → 放行；final 已发出 → 排队；否则 → 挂起。
    ///
    /// 能力（`judge.intent`）**不在本函数判**：它是**写权**，拦在落格那一侧
    /// （`inbox::commit_preemption` 的写侧闸），与 `decide` 是纯函数判定分开——
    /// 判定可以算，写控制事实才需要资格。
    pub fn decide(&self, spec: &ActorSpec, events: &[Event], elapsed_ms: u64) -> Preemption {
        if elapsed_ms > spec.budget_ms {
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
    /// - `source_seq` = **挂起前最后一条已落事实的 seq**（04 §2.2 步 1「记录当前
    ///   seq 为 T」）——它是幂等键的后半段，因此必须**每次挂起都不同**：同一个任务
    ///   可以被挂起多次（挂起 → 恢复 → 再挂起），拿任务开格 seq 当锚会让第二次撞
    ///   `Duplicate` 而**静默丢掉**那次挂起（J3）。
    ///
    ///   **不能取 `head`**：`head` 是"下一条的号"，它还不是事实。拿它当
    ///   `produced_by` 会让落格后那条边指向**自己**（`produced_by == seq`）。
    ///   I2 只查"有没有边"时这是绿的，`provenance_dangling` 上线后它必须红——
    ///   调用方因此传 `head - 1`（见 `inbox::append_preemption`）。
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

/// 闸门结论（S8 第 20 步，[roadmap/S09 §6](../../../../docs/007-task/129-外部执行与熔断.md ")）。
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

/// 熔断闸门（S8 第 20 步，[roadmap/S09](../../../../docs/007-task/129-外部执行与熔断.md ")）。
///
/// `budget_ms = 80` 的反射档判定：超预算 / 超时**熔断**而不是"先做了再说"。
/// 授权判定留在调用方（读路径持矩阵）——本体的输入是判据数据，不是 ⑥ 的句柄。
pub struct CircuitBreaker;

impl CircuitBreaker {
    /// 外部执行闸门。
    ///
    /// - `authorized`：主体是否持外部执行能力（`ProduceArtifact`，读路径判定）；
    /// - `spent_ms` / `requested_ms` / `budget_ms`：已耗 / 本次申请 / 总预算；
    /// - `elapsed_ms`：**判据输入已耗的时长**（从批首开始读事实源到调用点，
    ///   不是「调用本函数前那几条指令」）；超过 `elapsed_budget_ms` 即熔断
    ///   （**有事件**的超时，不是静默失效；I3）；
    /// - `elapsed_budget_ms`：这份输入的时延预算。**由调用方给**，因为它的正确取值
    ///   取决于输入是什么：纯查表可以用反射档（80ms），而本闸门的输入是**会话事实源
    /// 的整段重放**（O(事件数)），那是深度档的操作——相应批次 之前硬编码反射档，
    ///   于是判据只在「重放不到 80ms」时可达，长会话下反而永远不触发。
    ///
    /// 相应批次: ../../../../docs/007-task/186-虚闸清偿批.md "
    pub fn gate(
        &self,
        authorized: bool,
        spent_ms: u64,
        requested_ms: u64,
        budget_ms: u64,
        elapsed_ms: u64,
        elapsed_budget_ms: u64,
    ) -> GateDecision {
        if elapsed_ms > elapsed_budget_ms {
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

/// 淘汰判定者（S13 §3，[roadmap/S13 §2/§3](../../../../docs/007-task/133-多智能体社会.md ")）：
/// 声誉投影低于阈值 ⇒ 收回 `assign.work` 的**裁决侧**。
///
/// 与 [`PreemptionDecider`] 同形而同用：判定是**纯函数**（读视图出结论），落格是
/// **事件**（经 Store，无直连——I1）。三件事各住一边：
///
/// | 事 | 住哪 |
/// |---|---|
/// | 判据（阈值、低于即淘汰） | [`Self::should_eliminate`]（纯函数） |
/// | 那一格事件（`verdict × asserted`） | [`Self::elimination_event`] / [`Self::restoration_event`] |
/// | 什么时候判、对谁判 | 调用方（`plugins/session/derived_facts` 的轮末步） |
///
/// ## 为什么 actor 是仲裁者而不是被淘汰主体
///
/// 与 [`PreemptionDecider`] 的先例同一条理由：事件的 `actor` 是**判定者**——
/// `session/arbiter`（授权表里唯一持 `assert.verification` 的一行，C15 构造期保证
/// 它不持 `define.work` / `produce.artifact`，自成对安全）。被淘汰的主体是载荷里的
/// `subject`（数据），不是作者。把两者混同会让「谁做的裁决」在事实源里说不出来。
///
/// ## 判据的输入是投影不是自报
///
/// `score` 来自 `reputation` 投影（从 `commitment.*` 事件派生），主体**自报**的
/// 声誉事件进不了投影（断言 2：只认派生）——于是「自己报个好分数免于淘汰」
/// 在构造上不成立，不是靠纪律拦住。
///
/// ## 结构性的诚实缺口：自动恢复不可能
///
/// 淘汰后唯一能涨声誉的路径是**成功委派**（`agent_run` 守约 ⇒ `commitment.released`），
/// 而委派恰是被收回的 `assign.work` 那个动作——闸门会把它拒掉。于是恢复不可能
/// **自动**发生：申诉 / 人工复位必须经同一条事件通道写 `restore`（机制半边在
/// [`Self::restoration_event`]，由撤销回路单测证）。生产侧的申诉触发器是
/// 相应批次
/// 之后的事，此处不造一个「假装能自动恢复」的回路。
pub struct EliminationDecider;

impl EliminationDecider {
    /// 淘汰阈值（平凡值）：声誉分**低于**它即淘汰。
    ///
    /// 平凡打分 = 守约 − 违约（`plain_score`），阈值 0 ⇒ **任何未配平的违约**
    /// （净分为负）就触发。它是**参数不是架构**（S13 §4 的平凡值表同口径）：
    /// 换阈值不改本判定的形状；`should_eliminate` 是唯一读它的地方。
    ///
    /// **为什么不分档（ 项 13 的复核结论）**：分档要的是**分数上的梯度**，
    /// 而 `plain_score` 的取值只有计数差（一次立约了结 = ±1），没有任何一维能撑起
    /// 「警告档 / 观察档 / 收回档」的**后果差异**——今天唯一的后果就是收回
    /// `assign.work` 这一格（[D-108](../../../../docs/003-design/002-架构总览.md "D-108") 记的
    /// 单向门正是它的对面）。多分一档 = 多一个没有消费者的后果档位，而收回不可自动
    /// 恢复，档位只会把「何时被永久锁死」变成一处更难看清的参数。真要有梯度，先要有
    /// 第二种后果（分档的对象是后果，不是分数），那属 S13 社会面的裁决，不在此顺手加。
    pub const THRESHOLD: i64 = 0;

    /// S13 仲裁者**行**（`ActorSpec` 生产构造点，与 [`PreemptionDecider::spec`] 同批同形）。
    ///
    /// `{ principal: session/arbiter, pattern: decider, capabilities: [assert.verification],
    /// budget_ms: 80, scope: root }`——五个字段全部**取实**：principal / capabilities
    /// 同源于授权表那一行（ 落地的 `PRINCIPAL_ARBITER` 窄行），pattern 按
    /// [`Pattern::for_principal`] 派生（仲裁者无 `reply.*` 也无 `produce.artifact`
    /// ⇒ `Decider`，三新角色同一答案——**模式闭集不变宽**，exit ① 的另一半），
    /// budget = 反射档（淘汰判定是规则折叠，毫秒级），scope = root。
    pub fn spec(&self) -> ActorSpec {
        let principal = PRINCIPAL_ARBITER.to_string();
        ActorSpec {
            pattern: Pattern::for_principal(&principal),
            capabilities: crate::symbio_core::authz::production_matrix().grant_names_of(&principal),
            budget_ms: crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms(),
            scope: Scope::Root,
            principal,
        }
    }

    /// 纯判定：该主体的声誉条目是否低于淘汰阈值。
    ///
    /// 只问分数——「对谁判」（持 `assign.work` 的主体）、「判过没有」（无既有
    /// verdict）都由调用方定：判定者不持有授权表以外的任何判据（与
    /// [`CircuitBreaker::gate`] 的 `authorized` 出参同一条纪律）。
    pub fn should_eliminate(&self, entry: &ReputationEntry) -> bool {
        entry.score < Self::THRESHOLD
    }

    /// 淘汰事件（`verdict × asserted`，`change = "revoke"`）——可审计可撤销的那一格。
    ///
    /// 载荷六字段全为**数据**：`subject`（被收回的主体）、`capability`
    /// （收回的能力，恒为 `assign.work`——S13 §2 淘汰行的字面机制）、`change`、
    /// `score` / `threshold`（判据的当时值，审计时要能复算）、`why`（人话理由）。
    /// `source_seq` = 溯源锚（I2：断言类事件必须非空）。
    pub fn elimination_event(&self, subject: &str, score: i64, source_seq: u64) -> Event {
        Event::pending(
            format!("v-elim-{subject}-{source_seq}"),
            crate::symbio_core::event::EVENT_VERDICT_ASSERTED,
            crate::symbio_core::event::Entity::Verdict,
            crate::symbio_core::event::Verb::Asserted,
            0,
            PRINCIPAL_ARBITER,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "subject": subject,
            "capability": crate::symbio_core::authz::CAP_ASSIGN_WORK,
            "change": "revoke",
            "score": score,
            "threshold": Self::THRESHOLD,
            "why": format!("声誉分 {score} 低于淘汰阈值 {}（守约 − 违约，只认派生）", Self::THRESHOLD),
        }))
    }

    /// 恢复事件（`verdict × asserted`，`change = "restore"`）——淘汰的逆操作。
    ///
    /// 与 [`Self::elimination_event`] **同格同键形状**：恢复不是删历史（Log 永不删），
    /// 是再写一条 `restore`，last-wins 折叠出当前状态（`GrantsOverlay`）。申诉成立 /
    /// 人工复位走的都是这一格；`why` 由调用方给（申诉受理、人工复核……）。
    ///
    /// **为什么它今天没有生产调用方**（诚实缺口，不是漏接）：淘汰后唯一能涨声誉的
    /// 路径是成功委派，而委派恰是被收回的 `assign.work` 那个动作——闸门会拒掉它，
    /// 于是恢复不可能**自动**发生，必须有一个外部触发（申诉 / 人工复位）。这条缺席
    /// 已在 [D-108](../../../../docs/003-design/002-架构总览.md "D-108") 记为**边界**并写明重开
    /// 条件；触发器落地时摘掉这个属性即可，载荷口径不随之改动。
    /// 把它当成「已实现的恢复能力」是本条要拦的读法：形状是真的，写方是缺席的。
    #[cfg(test)]
    pub fn restoration_event(&self, subject: &str, why: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("v-rest-{subject}-{source_seq}"),
            crate::symbio_core::event::EVENT_VERDICT_ASSERTED,
            crate::symbio_core::event::Entity::Verdict,
            crate::symbio_core::event::Verb::Asserted,
            0,
            PRINCIPAL_ARBITER,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "subject": subject,
            "capability": crate::symbio_core::authz::CAP_ASSIGN_WORK,
            "change": "restore",
            "why": why,
        }))
    }

    /// `verdict.asserted` 事件流水 → [`GrantsChange`] 序列：**载荷 ↔ 变更的映射**。
    ///
    /// 淘汰回路的读侧半边：闸门（`plugins/session` 的工具执行层）拿不到裁决者本体，
    /// 它有的是事件切片——本函数就是「切片 → 覆盖层」之间唯一的那步翻译。
    /// 与 [`commitment_events`] 同形态（写侧的成格函数 / 读侧的翻译函数各住一边）。
    ///
    /// 认不出的 `change` 值（含缺失）⇒ **跳过该条**：不猜「多半是撤销吧」——
    /// 把笔误读成一次撤销等于把仲裁裁决静默改判，fail-closed 的方向是**不动**。
    pub fn verdict_grants_changes(events: &[Event]) -> Vec<GrantsChange> {
        let mut out = Vec::new();
        for e in events {
            if e.kind != crate::symbio_core::event::EVENT_VERDICT_ASSERTED {
                continue;
            }
            let Some(subject) = e.payload.get("subject").and_then(|v| v.as_str()) else {
                continue;
            };
            let capability = e
                .payload
                .get("capability")
                .and_then(|v| v.as_str())
                .unwrap_or(crate::symbio_core::authz::CAP_ASSIGN_WORK);
            match e.payload.get("change").and_then(|v| v.as_str()) {
                Some("revoke") => out.push(GrantsChange::Revoke {
                    principal: subject.to_string(),
                    capability: capability.to_string(),
                }),
                Some("restore") => out.push(GrantsChange::Restore {
                    principal: subject.to_string(),
                    capability: capability.to_string(),
                }),
                _ => {}
            }
        }
        out
    }
}
