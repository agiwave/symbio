//! 承诺登记者：立约 / 守约 / 违约 / 宣告都是 `commitment` 实体的一格事件（[roadmap/S08](../../../../docs/003-stage/008-多主体与对等承诺.md "ST-008-多主体与对等承诺")）。

use crate::symbio_core::Event;

/// 承诺登记者（S6 第 15 步，[roadmap/S08 §3](../../../../docs/003-stage/008-多主体与对等承诺.md "ST-008-多主体与对等承诺")）。
///
/// 立约 = `commitment` 实体的一格事件；本体的职责只是把「谁向谁承诺了什么 /
/// 是否履行」落成**普通事件**（经 Store，无直连——I1 的直接推论）。
/// 违约不是异常通道：`broken` 与 `released` 是同一格（`commitment × closed`）
/// 的两个名字，违约必须带 `why`（可观测，S08 §5）。
pub struct CommitmentKeeper;

impl CommitmentKeeper {
    /// 立约：`from` 向 `to` 承诺 `promise`。`source_seq` 是触发本次立约的事件
    /// （溯源锚；无触发场景传 0 并由调用方保证可解释）。
    pub fn offer(&self, id: &str, from: &str, to: &str, promise: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("c-offer-{id}"),
            crate::symbio_core::event::EVENT_COMMITMENT_OFFERED,
            crate::symbio_core::event::Entity::Commitment,
            crate::symbio_core::event::Verb::Opened,
            0,
            from,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "id": id, "from": from, "to": to, "promise": promise }))
    }

    /// 守约收束。
    pub fn release(&self, id: &str, from: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("c-close-{id}"),
            crate::symbio_core::event::EVENT_COMMITMENT_RELEASED,
            crate::symbio_core::event::Entity::Commitment,
            crate::symbio_core::event::Verb::Closed,
            0,
            from,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "id": id }))
    }

    /// 违约收束（**必须带 why**——违约可被观测是 T5 的全部前提）。
    pub fn breach(&self, id: &str, from: &str, why: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("c-close-{id}"),
            crate::symbio_core::event::EVENT_COMMITMENT_BROKEN,
            crate::symbio_core::event::Entity::Commitment,
            crate::symbio_core::event::Verb::Closed,
            0,
            from,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "id": id, "why": why }))
    }

    /// 对等宣告：把承诺状态告知协作方（`commitment.asserted`，
    /// `commitment × asserted` 格——声明仍是一条普通事件，带溯源）。
    pub fn declare(&self, id: &str, from: &str, statement: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("c-assert-{id}"),
            crate::symbio_core::event::EVENT_COMMITMENT_ASSERTED,
            crate::symbio_core::event::Entity::Commitment,
            crate::symbio_core::event::Verb::Asserted,
            0,
            from,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "id": id, "from": from, "statement": statement }))
    }
}

/// `commitment.*` 一族的**运行时入口**（[plan/04 §3.1 批⑧](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")，
/// NDC-001：定义域外不得提及主体类型名——提及即可持有，持有即可绕过事实源直连）。
///
/// 返回**待入格的事件序列**：立约（`opened`）→ 了结（`released` / `broken`）→
/// 必要时宣告（`asserted`）。写方（`plugins/session` 的轮末收束）只管按序 append，
/// 「什么时候落格」是它的决定，「怎么把一次立约变成格子」是本域的决定——与
/// [`recalled_event`] 同一形态。
///
/// 违约才宣告：守约无需告知（履行的东西就摆在那里），违约却必须让承诺对象知道
/// （S08 §5：违约可被观测是 T5 的全部前提）。宣告仍是同一份事实源里的**一格**，
/// 不是新通道（S08 §2「通信 = 没有直连」）。
///
/// 生产侧只准调本函数。
pub fn commitment_events(
    id: &str,
    from: &str,
    to: &str,
    promise: &str,
    ok: bool,
    why: &str,
    source_seq: u64,
) -> Vec<Event> {
    let keeper = CommitmentKeeper;
    let mut out = vec![keeper.offer(id, from, to, promise, source_seq)];
    if ok {
        out.push(keeper.release(id, from, source_seq));
    } else {
        out.push(keeper.breach(id, from, why, source_seq));
        out.push(keeper.declare(id, from, &format!("已告知 {to}：违约——{why}"), source_seq));
    }
    out
}
