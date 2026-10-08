//! 事实网格 → prompt 的**双向完整性**判据（[ADR-048「系统性失败的防线」节](../../../../docs/decisions/session.md#adr-048-出厂档位改-fullv2-链路是出厂路径bridge-退役)）。
//!
//! ## 它防的是什么
//!
//! 三条已证实的缺口（请求视图三段不传 / `context_messages = 0` 清空历史 / 折进的补充
//! 不进事实源）**全部静默**：不报错、不告警、既有断言全绿（事实照样入格、`session/stats`
//! 照样有数），只有 e2e 在换档位的那一瞬间偶然照到。本模块把「模型实际看到的东西」变成
//! **可断言的**，让这类缺口在单测层就红。
//!
//! ## 形态：为什么不是「两条路径对照」
//!
//! `bridge` 已退役（ADR-048），对照法失去对象。且对照法是**双向的**——两边都可能错，
//! 判据没有唯一一侧。这里反过来：**事实网格是唯一真源**，所以每条判据都以网格为基准。
//!
//! ## 三个方向
//!
//! | 方向 | 断言 | 照出什么 |
//! |---|---|---|
//! | 网格 → prompt | 每条投影内的事件都有一条对应 entry，且 `role` 正确 | 投影层丢事实 |
//! | `role` 完好 | `role == "tool"` 的 entry 必须带工具名 | 工具结果的机器可读关联 |
//! | prompt → 网格 | 每条 entry 都能在网格里找到出处 | 「模型看到了事实源没有的东西」= 两条真源 |
//!
//! ## 为什么它便宜到每轮都跑
//!
//! 纯函数 + 单测层：不起 CLI、不 mock LLM、不碰 IO。所以它不会被「太重」跳过——这一点
//! 是本条存在的意义，全量 e2e 反而做不到（每步都要重起进程、跑 42 个用例）。

use super::super::event::{
    Entity, Event, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE,
};
use super::transcript::TranscriptEntry;

/// 缺口的一条报告。**故意不 derive `PartialEq`**：报告要给门禁与日志读，字段顺序变动
/// 不该让「内容相同」的报告变成另一种类型。
#[derive(Debug, Clone)]
pub enum PromptGap {
    /// 网格里有一条投影内的事件，没有对应的 prompt entry（投影层把它丢了）。
    DroppedFromPrompt { kind: String, turn: u64 },
    /// prompt 里有一条 entry，网格里找不到出处（**两条真源**）。
    NoSourceInGrid { role: String, text: String },
    /// `role == "tool"` 却没带工具名——工具结果的机器可读关联丢了。
    ToolEntryWithoutName { text: String },
    /// 角色与网格里那条事件的坐标不符（投影把 user 说成 assistant 之类）。
    RoleMismatch {
        expected: &'static str,
        got: String,
        kind: String,
        turn: u64,
    },
}

/// 网格事件 → 它在投影里应当成为的 entry（`None` = 不在投影范围内）。
///
/// **唯一真源**：事件 kind 的集合只在这里列一次。投影若漏了某个 kind，本函数是第一个
/// 知道的——而不是等 prompt 里少了东西才从症状往回猜。
fn expected_role(e: &Event) -> Option<&'static str> {
    match (e.entity, e.verb, e.kind.as_str()) {
        (Entity::Turn, Verb::Opened, EVENT_USER_MESSAGE) => Some("user"),
        (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FINAL) => Some("assistant"),
        (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FALLBACK) => Some("assistant"),
        (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => Some("tool"),
        _ => None,
    }
}

/// 事件载荷里该进 prompt 的正文（与 `transcript` 投影**同一取法**——两处各写一遍必然漂移，
/// 所以这里只做「有没有」，正文比对交给 `SourceRef` 的坐标）。
fn payload_text(e: &Event) -> &str {
    match e.kind.as_str() {
        EVENT_ASSISTANT_FALLBACK => e.payload.get("why").and_then(|v| v.as_str()).unwrap_or(""),
        k if k == EVENT_ARTIFACT_ADDED => {
            e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("")
        }
        _ => e.payload.get("text").and_then(|v| v.as_str()).unwrap_or(""),
    }
}

/// 一次完整性复核的结果。
#[derive(Debug, Clone, Default)]
pub struct PromptReport {
    pub gaps: Vec<PromptGap>,
    /// 参与复核的网格事件数（投影范围内）。
    pub checked_events: usize,
    /// 参与复核的 prompt entry 数。
    pub checked_entries: usize,
}

impl PromptReport {
    /// 是否完整（无缺口）。
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }

    /// 缺口的人读摘要（门禁 / 测试失败消息用）。
    pub fn summary(&self) -> String {
        if self.is_complete() {
            return format!(
                "完整：{} 条网格事件 → {} 条 prompt 消息",
                self.checked_events, self.checked_entries
            );
        }
        let mut s = format!(
            "{} 处缺口（{} 条网格事件 / {} 条 prompt 消息）：\n",
            self.gaps.len(),
            self.checked_events,
            self.checked_entries
        );
        for g in &self.gaps {
            let one = match g {
                PromptGap::DroppedFromPrompt { kind, turn } => {
                    format!("网格→prompt 丢事实：{kind}（turn {turn}）没有对应消息")
                }
                PromptGap::NoSourceInGrid { role, text } => format!(
                    "prompt→网格 无出处：一条 role={role} 的消息在事实源里找不到对应（内容前 40 字：{:.40?}）",
                    text
                ),
                PromptGap::ToolEntryWithoutName { text } => format!(
                    "role 退化：role=tool 却没带工具名（内容前 40 字：{:.40?}）",
                    text
                ),
                PromptGap::RoleMismatch { expected, got, kind, turn } => format!(
                    "角色不符：{kind}（turn {turn}）应是 {expected}，prompt 里是 {got}"
                ),
            };
            s.push_str("  · ");
            s.push_str(&one);
            s.push('\n');
        }
        s
    }
}

/// **双向**完整性复核：网格 ↔ prompt 消息。
///
/// `events` = 事实网格的可见切片（窗口 / 可见域已在上游裁好），`entries` = 交给模型的那
/// 批消息。**入参不含「投影」**——本函数自己按 `expected_role` 重算一遍，这样它能同时
/// 照出「投影漏了」与「prompt 里有网格之外的」。
pub fn verify(events: &[Event], entries: &[TranscriptEntry]) -> PromptReport {
    let mut rep = PromptReport::default();

    // 网格侧：投影范围内的事件，逐条找对应 entry。
    let mut used = vec![false; entries.len()];
    for e in events {
        let Some(role) = expected_role(e) else {
            continue;
        };
        rep.checked_events += 1;
        let text = payload_text(e);
        // ⚠️ **只按正文配对，不把 role 放进配对键**——放进去了，「角色不符」就永远匹配
        // 不上，于是会被误报成「丢事实」。那两类缺口的区别恰恰是 role：正文在、角色错，
        // 与正文根本不在，是两种不同的病，判据必须分得开。
        //
        // 取**未用过**的第一条：同一轮可能有多个同正文事件（重试各成一格）；事件按 seq
        // 有序、prompt 消息也按序，于是「首个未用」保持原序配对。
        let hit = entries
            .iter()
            .enumerate()
            .position(|(i, m)| !used[i] && m.text == text);
        match hit {
            None => rep.gaps.push(PromptGap::DroppedFromPrompt {
                kind: e.kind.as_str().to_string(),
                turn: e.turn,
            }),
            Some(i) => {
                used[i] = true;
                let m = &entries[i];
                if m.role != role {
                    rep.gaps.push(PromptGap::RoleMismatch {
                        expected: role,
                        got: m.role.clone(),
                        kind: e.kind.as_str().to_string(),
                        turn: e.turn,
                    });
                }
                if m.role == "tool" && m.tool.as_deref().map(str::trim).unwrap_or("").is_empty() {
                    rep.gaps.push(PromptGap::ToolEntryWithoutName {
                        text: m.text.clone(),
                    });
                }
            }
        }
    }

    // prompt 侧：每条 entry 都得被网格认领过（未认领 = 网格之外的东西进了 prompt）。
    for (i, m) in entries.iter().enumerate() {
        rep.checked_entries += 1;
        if !used[i] {
            rep.gaps.push(PromptGap::NoSourceInGrid {
                role: m.role.clone(),
                text: m.text.clone(),
            });
        }
    }

    rep
}

#[cfg(test)]
#[path = "prompt_fidelity.test.rs"]
mod tests;
