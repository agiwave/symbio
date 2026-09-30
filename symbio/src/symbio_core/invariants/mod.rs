//! 三条不变量的可执行检查（v2 F4，[plan/01 §5](../../../../docs/plan/01-核心架构.md)、[plan/04 §4](../../../../docs/plan/04-工程落地.md)）。
//!
//! ## 形态：纯函数，不是 trait
//!
//! 每条检查 = 「事件切片 ⇒ 违规清单」。输入是数据，输出是数据——它可以被
//! 单测双跑、可以挂 CI、可以离线跑在导出的事件序列上，而不需要任何运行时。
//!
//! ## S0 覆盖的三条 CI 断言（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 步骤 2 的出口判据）
//!
//! | 断言 | 不变量 | 检查 | 函数 |
//! |---|---|---|---|
//! | C1 | I1 单通道 | `seq` 严格单调、无跳号 | [`seq_monotonic`] |
//! | C2 / N3 | I1 单通道 | 每 `turn` 至多 1 条 final | [`final_unique_per_turn`] |
//! | C3 / N5 | I2 无溯源不声明 | 断言类事件 `produced_by` 非空 | [`produced_by_coverage`] |
//!
//! ## 判定纪律
//!
//! 每条检查配**反向用例**：喂入违规序列，违规必须被看见；看不见 = 检查是摆设。

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

/// C3 / N5（I2）：断言类事件（`verb == Asserted`）必须带溯源。
///
/// 断言 = 「宣称某事实为真」（`task.verified` / `artifact.added` / `classify.verdict`…）。
/// 没有溯源的断言**无法审计也无法撤销**——静默失效的温床（J3）。
pub fn produced_by_coverage(events: &[Event]) -> Vec<Violation> {
    events
        .iter()
        .filter(|e| e.verb == Verb::Asserted && e.produced_by.is_none())
        .map(|e| Violation::at(e, "断言类事件必须带 produced_by（I2 无溯源不声明）"))
        .collect()
}

/// 三条一起跑（S0 的 CI 形态：任一违规 ⇒ 清单非空）。
pub fn check_all(events: &[Event]) -> Vec<Violation> {
    let mut all = seq_monotonic(events);
    all.extend(final_unique_per_turn(events));
    all.extend(produced_by_coverage(events));
    all
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
