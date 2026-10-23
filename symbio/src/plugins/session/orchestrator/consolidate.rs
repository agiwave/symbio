//! 压缩快照入格 + 归档（[D-086](../../../../../docs/003-design/014-单环一表.md "D-086") §4/§6）。
//!
//! 把「本轮之前发生过一次压缩」变成一格事实（`message × progressed` =
//! `session.consolidated`），并把被它取代的前缀移入归档段。
//!
//! ## 为什么写方是这里，而不是压缩现场
//!
//! 事实只能由**持写者令牌**的一方落格（[`EventWalStore`] 的单写者闸门）。压缩跑在
//! 驱动器内，**不持**令牌——令牌随 WAL 打开而取，而动作面（`session/<sid>/_run/
//! consolidate`）正是 session 侧持令牌的执行口。压缩把「压成了什么」交给动作，
//! 由本模块在**动作执行的那一刻**落格。
//!
//! ## 幂等：按**内容**比对，不按事件 id
//!
//! 同一轮里执行口可能被调多次，而压缩只发生一次 ⇒ 第二次不该再落一条。判据是与
//! 完整史里**最近一份**快照逐 id 比对（[`crate::symbio_core::mutations_latest_snapshot`]）：
//! 一致 ⇒ 这份已经落过了。用内容而不是事件 id 判，是因为「该不该落」这件事由
//! **压成了什么**决定，而事件 id 只是它的名字。
//!
//! ## 归档：快照即取代（b2 的语义）
//!
//! 落格成功后调 [`EventWalStore::archive_before`]，把 `seq < 快照` 的事件整段移入归档段。
//! **「上下文压缩」与「WAL 无界增长」因此由同一个机制解决**（D-053 决策 6）：一次压缩
//! = 多一条快照事件 + 把被取代的前缀移走。归档段不删——D-053 迁移表 b2 行的
//! 「零回退损失」正靠它（各段按号序拼接 + live，逐条不重不漏 = 原 WAL）。
//!
//! ## 载荷带**轮次水位**（`turns`）
//!
//! 归档把被取代的轮次移出了**现行窗口**（最近一份快照之后那段），而轮号数的是现行窗口
//! 里的 `user.message`（[`count_user_messages`] 取 `turns` 水位 + 其后计数）⇒ 不补水位，
//! 压缩之后轮号会**从 1 重新数起**，而 `u-{turn}` 是幂等键：重号会让新轮的用户格撞
//! `Duplicate` 静默落不进去。所以快照必须把「我取代了多少轮」一起带走——水位是历史
//! 状态的一部分，不是派生细节。
//!
//! ## 失败一律**不致命**
//!
//! 落格失败 ⇒ 告警并放弃归档（事实源保持完整）。归档失败 ⇒ 告警。两者都不回错给
//! 调用方：压缩**已经发生**（消息列表已重写、界面已变短），把它升级成整轮失败等于
//! 用「记不下来」换「答不出来」——与缺口 4 的汇报落格同一条口径。

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{
    clock_now_ms, count_user_messages, Entity, Event, Seq, Store, Verb, EVENT_SESSION_CONSOLIDATED,
};

use super::super::chat_session::PersistentChatSession;
use super::super::store::EventWalStore;

/// 压缩入格 + 归档；返回新落快照事件的 `seq`（`None` = 没有要落的，或这份已经落过）。
///
/// 入参是 `principal` 而不是 `ActorSpec`：本格只用到主体名，而两个调用点一个在轮内、
/// 一个在轮外——收窄到真正用到的那一项。
pub(crate) fn consolidate_into_wal(
    store: &EventWalStore,
    messages: &[ChatNode],
    principal: &str,
) -> Option<Seq> {
    let existing = store.range(Seq::new(0));
    if let Some((_, prev)) = crate::symbio_core::mutations_latest_snapshot(&existing) {
        if same_message_ids(&prev, messages) {
            return None;
        }
    }
    // 被取代前缀的末号 = 此刻已落的最后一条（新事件尚未落）。事件 id 取它：
    // 同一份历史只压一次 ⇒ 同一个 id ⇒ 重放撞幂等键（`Duplicate`）而不是多出一条
    // 快照；号空间单调 ⇒ 不同压缩点必不同 id。
    let head = store.head().value();
    let superseded_last = head.saturating_sub(1);
    // 水位 = 被取代前缀里数得出的轮数（**同一个** `count_user_messages`，故套娃压缩
    // 会逐层累加：上一份快照的 `turns` 已计入它的前缀，本份自然接得上）。
    let turns = count_user_messages(&existing);
    let event = Event::pending(
        format!("v2sc-{superseded_last}"),
        EVENT_SESSION_CONSOLIDATED,
        Entity::Message,
        Verb::Progressed,
        // `turn = 0`：压缩不属于任何一轮——它发生在轮**边界**上，被取代的是**多轮**的
        // 历史（与 `memory.*` 同一约定，见 plan/01 §6）。
        0,
        principal,
    )
    .with_ts(clock_now_ms())
    .with_payload(serde_json::json!({
        "messages": messages,
        "turns": turns,
    }));
    // 溯源锚 = 被取代前缀的末号（「这份快照是在那段历史之上压出来的」）。空 WAL
    // （迁移前的历史只存在于 `messages.json`）时**无锚可指** ⇒ 不设：I2 不要求
    // `progressed` 带溯源，编一个 `0` 反而是假锚。
    let event = if head > 0 {
        event.with_produced_by(superseded_last)
    } else {
        event
    };
    let seq = match store.append(event) {
        Ok(seq) => seq,
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[Session] 压缩入格失败（压缩已生效，事实源缺这一格）: {e:?}"
            );
            return None;
        }
    };
    // 快照即取代：被取代的前缀移入归档段（不删）。失败**不回错**——归档是
    // 增长优化、不是正确性前提；没被取代的那段一字未动（见 `archive_before` 的崩溃安全说明）。
    match store.archive_before(seq) {
        Ok(_) => {}
        Err(e) => crate::plugin_warn!(
            "session",
            "[Session] 压缩归档失败（事实源保持完整，仅未缩短）: {e}"
        ),
    }
    Some(seq)
}

/// 两份消息列表的 id 序列是否一致（压缩入格的幂等判据）。
///
/// **只比 id、不比正文**：快照的语义是「这份列表**取代**了它之前的一切」——判据是
/// **身份**，不是内容。正文在快照落格之后仍可被 `message.edited` 改（那是另一条事实），
/// 把它也算进判据会让「同一份快照」在编辑之后不再相等。
fn same_message_ids(a: &[ChatNode], b: &[ChatNode]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.id == y.id)
}

/// **轮外**压缩落格：给 `context::retry_compaction` 用。
///
/// ## 为什么它需要一条自己的路
///
/// 用户重试压缩跑在 turn 循环**之前**（`process_resume` 在循环前调用它，之后一律
/// `Done`），所以没有「下一个执行轮」可以寄存——那条路唯一能落格的时刻就是它自己。
/// 此刻无人持写者令牌（循环还没开始），于是自己开一次 WAL 并交给
/// [`consolidate_into_wal`]——**落格实现仍是同一个**，差别只在谁持令牌。
///
/// ## 失败只告警
///
/// 打不开 / 拿不到令牌（同会话上有并发请求）⇒ 只告警并放弃。口径与轮内一致：压缩
/// **已经生效**（消息列表已重写、界面已变短），把它升级成整轮失败等于用「记不下来」
/// 换「答不出来」。
pub(crate) fn consolidate_outside_turn(
    session: &PersistentChatSession,
    messages: &[ChatNode],
    principal: &str,
) {
    let store = match session.facts() {
        Ok(Some(store)) => store,
        Ok(None) => return,
        Err(why) => {
            // ：打不开与「没有事实源」是两种处置——前者要出声（压缩已生效、
            // 事实源缺这一格），后者（临时会话）本就无事可落。
            crate::plugin_warn!(
                "session",
                "[Session] 轮外压缩入格：事实源打不开（压缩已生效，事实源缺这一格）: {why}"
            );
            return;
        }
    };
    consolidate_into_wal(&store, messages, principal);
}

#[cfg(test)]
#[path = "consolidate.test.rs"]
mod tests;
