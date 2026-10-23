//! 承诺登记者：立约 / 守约 / 违约 / 宣告都是 `commitment` 实体的一格事件（[roadmap/S08](../../../../docs/001-goal/128-多主体与对等承诺.md "GO-128-多主体与对等承诺")）。

use crate::symbio_core::Event;

/// 承诺登记者（S6 第 15 步，[roadmap/S08 §3](../../../../docs/001-goal/128-多主体与对等承诺.md "GO-128-多主体与对等承诺")）。
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

/// `commitment.*` 一族的**运行时入口**（[plan/04 §3.1 批⑧](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")，
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

/// 一次立约读回来的一条记录（[`commitment_records`] 的元素）。
///
/// 字段就是 `offer` 当初写进载荷的那几样（`id` / `from` / `to` / `promise`）加上
/// 了结的形状；`why` 只有违约那条才有。`state` 的三个取值是本函数的**输出词表**，
/// 不是事件名——读方拿到的是「这一约怎么样了」，不是「哪一格落了笔」。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CommitmentRecord {
    /// 承诺号（载荷 `id`；轮末写方给的是 `v2c-{轮干}-{tool_call_id}`，跨轮不撞）。
    pub id: String,
    /// 承诺方（声誉记在它头上）。
    pub from: String,
    /// 承诺对象。
    pub to: String,
    /// 承诺了什么（`agent_run` 的那句 prompt）。
    pub promise: String,
    /// `kept` / `broken` / `open`（立了还没了结）。
    pub state: &'static str,
    /// 违约原因；非违约为空串。
    pub why: String,
    /// 溯源锚 = 立约那一轮开口的 seq（`produced_by`，I2）。
    pub anchor_seq: u64,
}

/// `commitment.*` 一族的**读侧翻译**：事件切片 → 一条一条的立约记录。
///
/// ## 为什么住在 core 而不是读方
///
/// 与 [`EliminationDecider::verdict_grants_changes`] 同形态——**写侧的成格函数与读侧的
/// 翻译函数各住一边，两头都不许自己另发明一套配对规则**：配对靠的是载荷 `id`
/// （[`CommitmentKeeper`] 写进去的），那是本域的形状知识。插件出口只做取数与排版。
///
/// ## 三条口径
///
/// - **一条约 = 一个载荷 `id`**：`opened` 起一条记录，`released` / `broken` 给它收尾；
///   `asserted`（违约时对协作方的告知）是**同一条约的第三格**，不另计一条——按格子数
///   除以 2 或 3 都会在守约/违约混合时算错；
/// - **认不出的了结 ⇒ 跳过**：载荷 `id` 找不到对应立约（切片被截断、或写方漏了
///   `opened`）时不猜「大概履行了吧」——与 `verdict_grants_changes` 同一条 fail-closed；
/// - **按 `id` 字典序输出**（`BTreeMap` 折叠）：同一份事实源永远读出同一个清单，
///   与 `reputation` / `readyset` 的确定性口径同族（N1 双跑一致）。
pub fn commitment_records(events: &[Event]) -> Vec<CommitmentRecord> {
    let mut by_id: std::collections::BTreeMap<String, CommitmentRecord> =
        std::collections::BTreeMap::new();
    for e in events {
        if e.entity != crate::symbio_core::event::Entity::Commitment {
            continue;
        }
        let Some(id) = e.payload.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        match e.kind.as_str() {
            crate::symbio_core::event::EVENT_COMMITMENT_OFFERED => {
                let str_of = |k: &str| {
                    e.payload
                        .get(k)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                by_id.entry(id.to_string()).or_insert(CommitmentRecord {
                    id: id.to_string(),
                    from: str_of("from"),
                    to: str_of("to"),
                    promise: str_of("promise"),
                    state: "open",
                    why: String::new(),
                    anchor_seq: e.produced_by.unwrap_or(0),
                });
            }
            crate::symbio_core::event::EVENT_COMMITMENT_RELEASED
            | crate::symbio_core::event::EVENT_COMMITMENT_BROKEN => {
                if let Some(rec) = by_id.get_mut(id) {
                    let broken = e.kind == crate::symbio_core::event::EVENT_COMMITMENT_BROKEN;
                    rec.state = if broken { "broken" } else { "kept" };
                    rec.why = if broken {
                        e.payload
                            .get("why")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    } else {
                        String::new()
                    };
                }
            }
            // `asserted`：违约的告知，同一条约的第三格。
            _ => {}
        }
    }
    by_id.into_values().collect()
}
