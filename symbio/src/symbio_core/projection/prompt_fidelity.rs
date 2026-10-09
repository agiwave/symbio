//! 事实网格 → prompt 的**双向完整性**判据（[ADR-048](../../../../docs/decisions/session.md#adr-048-出厂档位改-fullv2-链路是出厂路径bridge-退役)）。
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
    EVENT_ASSISTANT_REPORTED, EVENT_TURN_SUPPLEMENTED, EVENT_USER_MESSAGE,
};

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
    /// **按设计不进请求包**的事件（界面文本）漏进了送模型的那批消息。
    ///
    /// 为什么独立成一条而不并进 DroppedFromPrompt：两者方向相反——那条是
    /// 「该有的没有」，这条是「不该有的有了」。合成一个方向就会漏掉其中一半，
    /// 而漏掉的那一半恰好是「界面文本污染了对话」这种用户直接看得见的退化。
    ExcludedLeakedIntoPrompt { kind: String, turn: u64 },
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
        // 轮边界折进的补充（缺口 3）：它**是用户说的话**，所以 role=user。
        //
        // (P3c 补记) 这一行是补的——补充落进投影时忘了登记进这里，于是
        // expected_role 返回 None、判据把它当成「不在投影范围内」直接跳过。
        // 后果很隐蔽：**补充漏进请求包时判据不会红**。凡是落进投影的 kind 都必须
        // 在这里登记——判据的覆盖范围要与投影的实现范围同时长。
        (Entity::Turn, Verb::Asserted, EVENT_TURN_SUPPLEMENTED) => Some("user"),
        (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FINAL) => Some("assistant"),
        (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FALLBACK) => Some("assistant"),
        (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => Some("tool"),
        _ => None,
    }
}

/// **按设计不进请求包**的事件 kind（界面文本）。
///
/// 与 expected_role 是**互补**的两张表而不是一张表的两种取值：那张回答
/// 「它该是什么角色」，这张回答「它根本不该出现」。合成一张的话，「不该出现」
/// 就得靠某个假角色表达，而假角色会一路传到线格式去。
fn excluded_from_prompt(e: &Event) -> bool {
    matches!(
        (e.entity, e.verb, e.kind.as_str()),
        (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_REPORTED)
    )
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
    /// 参与复核的 prompt 消息数。
    pub checked_entries: usize,
    /// **被显式声明**为「按设计不进网格」的消息数（当前只有 `prefix` 三段读视图）。
    pub declared: usize,
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
                "完整：{} 条网格事件 → {} 条 prompt 消息（声明 {} 条请求级）",
                self.checked_events, self.checked_entries, self.declared
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
                PromptGap::ExcludedLeakedIntoPrompt { kind, turn } => format!(
                    "界面文本泄漏：{kind}（turn {turn}）标了「不进请求包」，却出现在送模型的消息里"
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

/// **双向**完整性复核：网格 ↔ **送进模型的消息**。
///
/// `events` = 事实网格的可见切片（窗口 / 可见域已在上游裁好），`messages` = 交给模型
/// 的那批消息。**入参不含「投影」**——本函数自己按 `expected_role` 重算一遍，这样它能
/// 同时照出「投影漏了」与「prompt 里有网格之外的」。
///
/// 验的是**送出去的那批**（含 prefix 与本轮工具交换），不是投影中间产物——
///
/// 投影绿 ≠ 模型看得全。缺口 1 / 缺口 3 都发生在投影**之外**：前者是请求视图三段没进
/// 投影（所以判据看不见它），后者是补充根本没成为事实（网格里就没有它，判据同样看不见）。
/// 验「送出去的」能多抓住一类：**投影有、但组包时丢了**。
///
/// ## `declared`：按设计不进网格的内容
///
/// 请求视图三段（记忆召回 / 就绪任务 / 委派者真源）是**读视图**——本次请求临时投影出来的，
/// 不是事件，所以它们**理应**报「无出处」。但把已知的那一条放过，判据才有用：
/// **一个永远红的守卫等于没有守卫。**
///
/// 放过是**声明**（这个参数），不是「判据放宽」——声明之后判据仍会报「多了一条无出处
/// 消息」，只是不再把**已知的那一条**算成缺口，而且 `Report::declared` 会把条数记下来，
/// 于是「例外有没有悄悄变多」是看得见的。
pub fn verify(
    events: &[Event],
    messages: &[crate::symbio_core::PromptMessage],
    declared: Option<&str>,
) -> PromptReport {
    let mut rep = PromptReport::default();

    // 网格侧：投影范围内的事件，逐条找对应 entry。
    let mut used = vec![false; messages.len()];
    for e in events {
        // 按设计不进请求包的那一类：**方向反过来**——不是「网格有、prompt 没有」，
        // 而是「网格标了不该进、prompt 里却出现了」。同样按正文配对，找得到就报缺口。
        //
        // 这条判据的存在理由与 `declared` 同源：漏进请求包的界面文本会让线上出现
        // 连续两条 assistant，而**没有任何现有判据会红**——`expected_role` 对它返回
        // None（它在投影里，但它不是对话内容）。
        if excluded_from_prompt(e) {
            rep.checked_events += 1;
            let text = payload_text(e);
            if messages.iter().any(|m| m.text == text) {
                rep.gaps.push(PromptGap::ExcludedLeakedIntoPrompt {
                    kind: e.kind.as_str().to_string(),
                    turn: e.turn,
                });
            }
            continue;
        }
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
        let hit = messages
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
                let m = &messages[i];
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

    // prompt 侧：每条消息都得被网格认领过，或被**显式声明**为请求级内容。
    for (i, m) in messages.iter().enumerate() {
        rep.checked_entries += 1;
        if used[i] {
            continue;
        }
        // 声明过的例外（如 `prefix` 三段读视图）：放过，但**记一笔**——
        // 放过本身不可见，例外就会悄悄变多。写进 `declared` 让它每次都看得见。
        if declared.is_some_and(|d| d == m.text) {
            rep.declared += 1;
            continue;
        }
        rep.gaps.push(PromptGap::NoSourceInGrid {
            role: m.role.clone(),
            text: m.text.clone(),
        });
    }

    rep
}

#[cfg(test)]
#[path = "prompt_fidelity.test.rs"]
mod tests;
