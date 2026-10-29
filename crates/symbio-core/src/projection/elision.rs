//! 区间省略的**唯一应用点**：请求包里「这一段不读」只有这一份判据。
//!
//! ## 一句话
//!
//! 组装 prompt 时「哪条事实不进请求包」有两个来源，而它们必须是**同一条事实形状**：
//!
//! | 来源 | 落在哪里 | 谁写 |
//! |---|---|---|
//! | 显式区间（「这段被摘要取代」） | `message.added` 载荷里的 `elision: {from, to}` | 任何能力（压缩行、摘要行） |
//! | 分支（「这一支没被选中」） | `turn.branch` / `turn.branch_selected` 折出的未选根 | 控制面 |
//!
//! 前者是**一条事件事实**，后者是**既有分支事实的一次折叠**——两者的应用点必须是同
//! 一个函数（[`Elisions::elides`]），否则「某段不读」会有两份答案，而分叉是静默的：
//! 转写里藏了、复核里还认领，或反过来，两种情况都不会红。
//!
//! ## 为什么区间载荷挂在 `message.added` 上，而不是新开一个事件
//!
//! 一次压缩是一次**原子陈述**：「这段历史被这一份取代」——取代物（`messages`）与被取代
//! 的区间（`elision`）在同一条事实里，读者不可能只读到一半。拆成两条事件就要靠一个
//! 连接键再拼回来，而「两条事实之间的一致性」正是事实网格不给保证的东西。
//!
//! `message.added` 的既有含义（D-054 决策 1：没有别的格可搭车的节点在这里诞生）因此
//! 扩了一档：**带 `elision` 的诞生格，载荷里的节点是取代那段历史的正文**，与界面卡片
//! （不带 `elision`）分得很清——[`is_replacement`] 是唯一的判别位。
//!
//! ## 区间的口径是**存储分配的 seq**（闭区间）
//!
//! `from` / `to` 是事件的 `seq`（`Seq::value()`），不是轮号、不是下标：seq 是网格里
//! 唯一全序且稳定的位置，切片被裁过、被归档过都还成立；轮号不唯一（一轮多条），下标
//! 随切片漂移。**`seq: None` 的事件（只存在于夹具 / 未落盘的行）不会命中显式区间**——
//! 它们本来就没有可被指认的位置。
//!
//! ## 纯函数、不产投影
//!
//! 本模块不含 `Projection::new`：它是 `transcript` 折叠与 `prompt_fidelity` 复核**共用**
//! 的读侧（与 `mutations` / `branch` 同族）。投影 A 不读投影 B 的输出（L2），所以这一份
//! 判据只有一个实现、两个调用点。

use std::collections::HashSet;

use super::super::event::Event;
use super::vocab::{slot_of, ChatSlot};
use crate::ChatVocab;
use crate::MessageVocab;

/// 一次切片里所有「不读」的落点：显式区间 + 未选中的分支。
#[derive(Debug, Clone, Default)]
pub(crate) struct Elisions {
    /// 显式区间：按事件 `seq` 的闭区间 `(from, to)`。
    ranges: Vec<(u64, u64)>,
    /// 分支事实里未被选中的支根 id（事件载荷 `turn_id`）。
    hidden_roots: HashSet<String>,
}

impl Elisions {
    /// 收一次切片的两类落点。
    ///
    /// `removed` 由调用方交进来（通常是 `MessageMutations::is_removed`）：分支「还在不在世」
    /// 的证据与转写折叠共用同一份索引——各算一遍会给出两种「哪一支还活着」。
    pub(crate) fn collect(
        events: &[Event],
        c: &ChatVocab,
        m: &MessageVocab,
        removed: impl Fn(&str) -> bool,
    ) -> Self {
        let mut ranges = Vec::new();
        for e in events {
            if slot_of(e, c, m) != Some(ChatSlot::MessageAdded) {
                continue;
            }
            if let Some(range) = elision_range(e) {
                ranges.push(range);
            }
        }
        Self {
            ranges,
            hidden_roots: super::branch::unselected_roots(events, removed, c),
        }
    }

    /// 这条事件读不读：落在任何显式区间里，或属于未选中的那一支 ⇒ 不读。
    pub(crate) fn elides(&self, e: &Event) -> bool {
        if let Some(seq) = e.seq.map(|s| s.value()) {
            if self
                .ranges
                .iter()
                .any(|(from, to)| *from <= seq && seq <= *to)
            {
                return true;
            }
        }
        super::branch::event_branch_root(e).is_some_and(|root| self.hidden_roots.contains(root))
    }
}

/// 这条诞生格是不是一次「区间被取代」（载荷带 `elision`）。
///
/// 投影与复核**共用这一个判别位**：它决定载荷里的节点是「取代旧区间的正文」（按节点
/// 自己的可见性进请求包）还是「界面卡片」（恒不进请求包）。
pub(crate) fn is_replacement(e: &Event) -> bool {
    e.payload.get("elision").is_some()
}

/// `elision` 载荷 → 闭区间（`from > to` 或非正整数一律视为没有省略：容错方向是
/// 「少藏」而不是「多藏」——多藏一段是模型看不见的话，而它不会报错）。
fn elision_range(e: &Event) -> Option<(u64, u64)> {
    let el = e.payload.get("elision")?;
    let from = el.get("from")?.as_u64()?;
    let to = el.get("to")?.as_u64()?;
    (from <= to).then_some((from, to))
}

#[cfg(test)]
#[path = "elision.test.rs"]
mod tests;
