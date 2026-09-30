//! 可执行不变量检查（v2 F4，[plan/01 §5](../../../../docs/plan/01-核心架构.md)、[plan/04 §4](../../../../docs/plan/04-工程落地.md)）。
//!
//! ## 形态：纯函数，不是 trait
//!
//! 每条检查 = 「事件切片 ⇒ 违规清单」。输入是数据，输出是数据——它可以被
//! 单测双跑、可以挂 CI、可以离线跑在导出的事件序列上，而不需要任何运行时。
//!
//! ## 覆盖的 CI 断言（S0 步骤 2 出口判据 + S1 步骤 4 故障注入，[roadmap/S01 §5](../../../../docs/plan/roadmap/S01-最小闭环.md)）
//!
//! | 断言 | 不变量 | 检查 | 函数 |
//! |---|---|---|---|
//! | C1 | I1 单通道 | `seq` 严格单调、无跳号 | [`seq_monotonic`] |
//! | C2 / N3 | I1 单通道 | 每 `turn` 至多 1 条 final | [`final_unique_per_turn`] |
//! | C3 / N5 | I2 无溯源不声明 | 断言类 + 收束类事件 `produced_by` 非空 | [`produced_by_coverage`] |
//! | C4 | I3 到点必答 | 开过的 turn 必须收束（final 或 fallback） | [`unresolved_turns`] |
//! | C5 | I3 到点必答 | 事件 `cost_ms` 不得超主体预算 | [`budget_exceeded`] |
//!
//! ## 判定纪律
//!
//! 每条检查配**反向用例**：喂入违规序列，违规必须被看见；看不见 = 检查是摆设。
//! 故障注入用例集（S1 步骤 4）在 `mod.test.rs` 的 fault 区统一收纳。

use super::event::{Entity, Event, Verb};

/// 一处违规：事件（按 seq 定位）+ 人话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// 违规事件的 `event_id`（seq 尚未分配的pending 事件用它定位）。
    pub event_id: String,
    pub why: String,
}

impl Violation {
    fn at(event: &Event, why: impl Into<String>) -> Self {
        Violation {
            event_id: event.event_id.clone(),
            why: why.into(),
        }
    }
}

/// C1（I1）：`seq` 严格单调且**无跳号**。
///
/// 已入库事件按存储顺序应得到 `0,1,2,…`——跳号意味着「有事件被静默丢弃或绕过
/// 唯一写入口」。`pending`（未入库）事件跳过不查（它们还没有 seq）。
pub fn seq_monotonic(events: &[Event]) -> Vec<Violation> {
    let mut bad = Vec::new();
    let mut expect = 0u64;
    for e in events {
        // pending（未入库）没有 seq，跳过——它们还不是事实。
        if let Some(s) = e.seq {
            if s.value() != expect {
                bad.push(Violation::at(
                    e,
                    format!("seq {} 处应为 {expect}（严格单调且无跳号）", s.value()),
                ));
                expect = s.value() + 1;
            } else {
                expect += 1;
            }
        }
    }
    bad
}

/// C2 / N3（I1）：每 `turn` 至多 1 条 final。
///
/// 「final」= `turn × closed` 格子上的 `chat.assistant.final` 名字（[plan/01 §6](../../../../docs/plan/01-核心架构.md)）。
/// 一轮对用户说两次「最终答复」，转写的收敛性就没了——这是 I1 在发言路径上的形状。
pub fn final_unique_per_turn(events: &[Event]) -> Vec<Violation> {
    let mut seen: Vec<(u64, &str)> = Vec::new();
    let mut bad = Vec::new();
    for e in events {
        let is_final =
            e.entity == Entity::Turn && e.verb == Verb::Closed && e.kind == "chat.assistant.final";
        if !is_final {
            continue;
        }
        if let Some((_, first_id)) = seen.iter().find(|(turn, _)| *turn == e.turn) {
            bad.push(Violation::at(
                e,
                format!(
                    "turn {} 已有 final（{}），又出现一条——每 turn 至多 1 条 final",
                    e.turn, first_id
                ),
            ));
        } else {
            seen.push((e.turn, e.event_id.as_str()));
        }
    }
    bad
}

/// C3 / N5（I2）：**声明类**事件必须带溯源。
///
/// 「声明」= 宣称某件事为真或已发生，三类（[plan/03 §2 I2](../../../../docs/plan/03-演进与验证.md)）：
/// - **断言类**（`verb == Asserted`）：`task.verified` / `artifact.added` / `classify.verdict`…
/// - **收束类**（`turn × closed` 的 `chat.assistant.final` / `chat.assistant.fallback`）：
///   宣称"已经答复"（[roadmap/S01 §5](../../../../docs/plan/roadmap/S01-最小闭环.md)）；
/// - **记忆类**（`entity == memory`，S5）：宣称"学到了什么"（[roadmap/S06](../../../../docs/plan/roadmap/S06-长期记忆与语义检索.md)
///   步骤 11 出口判据：溯源覆盖 100%——无溯源的记忆无法审计，也无法在遗忘时被追溯）。
///
/// 没有溯源的声明**无法审计也无法撤销**——静默失效的温床（J3）。
pub fn produced_by_coverage(events: &[Event]) -> Vec<Violation> {
    events
        .iter()
        .filter(|e| needs_provenance(e) && e.produced_by.is_none())
        .map(|e| {
            let why = if e.verb == Verb::Asserted {
                "断言类事件必须带 produced_by（I2 无溯源不声明）"
            } else if e.entity == Entity::Memory {
                "记忆事件必须带 produced_by（I2：溯源覆盖 100%）"
            } else {
                "收束类事件（答复）必须带 produced_by（I2：用户收到的话必须可追溯）"
            };
            Violation::at(e, why)
        })
        .collect()
}

/// I2 的覆盖判据（参照 [`docs/plan/verify/invariants.rs`](../../../../docs/plan/verify/invariants.rs)
/// 的 `needs_provenance`；S1 扩展收束类，S5 扩展记忆类）。
fn needs_provenance(e: &Event) -> bool {
    e.verb == Verb::Asserted
        || e.entity == Entity::Memory
        || (e.entity == Entity::Turn
            && e.verb == Verb::Closed
            && matches!(
                e.kind.as_str(),
                crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                    | crate::symbio_core::event::EVENT_ASSISTANT_FALLBACK
            ))
}

/// 三条一起跑（S0 的 CI 形态：任一违规 ⇒ 清单非空）。
pub fn check_all(events: &[Event]) -> Vec<Violation> {
    let mut all = seq_monotonic(events);
    all.extend(final_unique_per_turn(events));
    all.extend(produced_by_coverage(events));
    all
}

/// C4（I3 到点必答）：开过的 turn 必须收束。
///
/// 「收到」与「发出」是一对端点（[roadmap/S01 §1](../../../../docs/plan/roadmap/S01-最小闭环.md)）：
/// 有 `user.message`（`turn × opened`）却始终等不到 `chat.assistant.final` /
/// `chat.assistant.fallback`（`turn × closed`），就是**对话静默中断**——超时后
/// 什么都没发生，没有任何错误信号的那类失效。兜底必须产生事件（I3）。
pub fn unresolved_turns(events: &[Event]) -> Vec<Violation> {
    let mut bad = Vec::new();
    let mut opened: Vec<(u64, &str)> = Vec::new();
    for e in events {
        if e.entity == Entity::Turn && e.verb == Verb::Opened && e.kind == "user.message" {
            opened.push((e.turn, e.event_id.as_str()));
        }
        if e.entity == Entity::Turn
            && e.verb == Verb::Closed
            && matches!(
                e.kind.as_str(),
                crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                    | crate::symbio_core::event::EVENT_ASSISTANT_FALLBACK
            )
        {
            opened.retain(|(turn, _)| *turn != e.turn);
        }
    }
    for (turn, first_id) in opened {
        bad.push(Violation::at(
            events.last().expect("opened 非空 ⇒ 事件序列非空"),
            format!(
                "turn {turn}（自 {first_id} 开启）始终未收束——无 final 也无 fallback，对话静默中断（I3 到点必答）"
            ),
        ));
    }
    bad
}

/// C5（I3 到点必答）：事件的 `cost_ms` 不得超过主体预算。
///
/// 超预算本身**允许发生**（S4 的兜底链路负责降级），但它必须被**看见**：
/// `budget_ms` 是 I3 的记账口径，超了却没人知道 = 声明式预算（S1 之前的形态）。
pub fn budget_exceeded(events: &[Event], budget_ms: u64) -> Vec<Violation> {
    events
        .iter()
        .filter(|e| e.cost_ms > budget_ms)
        .map(|e| {
            Violation::at(
                e,
                format!(
                    "cost_ms {} 超出主体预算 {budget_ms}（I3 到点必答）",
                    e.cost_ms
                ),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
