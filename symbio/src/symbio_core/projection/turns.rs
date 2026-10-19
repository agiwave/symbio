//! 轮次水位投影：事实网格 → 已开轮数（含压缩快照的 `turns` 水位）。
//!
//! 住 core 的判据见 [ADR-086](../../../../docs/010-adr/086.md "ADR-086") §7：
//! session 的 consolidate 动作与 loop 的轮号分配读同一份口径；两处各写一次，
//! 压缩之后轮号就会在某一侧倒退（`u-{turn}` 幂等键撞号是**静默**的）。

use crate::symbio_core::event::{Entity, Event, Verb};
use crate::symbio_core::EVENT_USER_MESSAGE;

/// WAL 内 `user.message` 计数（新开轮的 turn 号；与桥同一口径）。
///
/// ## 为什么必须加上快照水位（S2.7-b2）
///
/// 压缩把被取代的轮次**移进了归档段**，而归档后的**现行窗口**里可能一条
/// `user.message` 都不剩（只剩快照 + 本轮新落的事件）⇒ 只数窗口会让轮号**倒退**：
/// 压缩之后从 1 重新数起。而 `u-{turn}` 是幂等键，重号会让新轮的用户格撞
/// `Duplicate`——那是**静默**的（那一轮在事实网格里没有用户格，下一轮 prompt 里
/// 整段消失）。所以水位必须随快照一起带过来：快照载荷的 `turns` = 它取代了多少轮。
///
/// 口径仍是「数 `user.message`」，只是分成「快照之前（读它的 `turns`）」+
/// 「快照之后（数窗口内事件）」两段相加——套娃压缩因此逐层累加，不会丢也不会重。
/// 前缀进了切片也不影响结果：折叠从**最近一份快照的下标**起，那段在它之前。
pub fn count_user_messages(events: &[Event]) -> u64 {
    let (start, base) = match super::mutations::mutations_latest_snapshot(events) {
        Some((i, _)) => (i + 1, consolidated_turns(&events[i])),
        None => (0, 0),
    };
    base + events[start..]
        .iter()
        .filter(|e| {
            e.entity == Entity::Turn && e.verb == Verb::Opened && e.kind == EVENT_USER_MESSAGE
        })
        .count() as u64
}

/// 快照事件载荷里的**轮次水位**（`session.consolidated` 的 `turns`）。
///
/// 缺失 / 非数 ⇒ `0`：那是旧版本写的快照（水位字段是 b2 才加的）。降级方向是
/// 「轮号从头数起」而不是「算出一个错的水位」——宁可保守。
pub(crate) fn consolidated_turns(e: &Event) -> u64 {
    e.payload.get("turns").and_then(|v| v.as_u64()).unwrap_or(0)
}
