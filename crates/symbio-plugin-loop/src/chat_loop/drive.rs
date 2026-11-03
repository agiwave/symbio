//! 接缝上的两次落格：`turn.dispatch` 与 `chat.assistant.opening`。
//!
//! 幕锚（用户格 / 恢复格 / 轮次格）已搬到 `chat_loop::open_turn` 的两行
//! （`open-turn` / `resume-turn`，D-313 P2-1）。本文件只剩**接缝**：两者都是
//! 幕级事实，内容要到轮首四行跑完才存在（档位由技能行作答后宿主代落的
//! `stage:"reflex"` 格折出，开场白是判决行 `Escalate` 首响的那句话），所以它们
//! 不能跟幕锚一起落，也不能留在 `TurnRunner::begin` 里。
//!
//! 为什么是自由函数而不是 `TurnSession::set_*`：这两格草稿一格都不读，能落格的
//! 句柄在接缝这一侧（事实源 + 上线面）。挂到草稿上等于让草稿持一份自己用不到的
//! 句柄。

use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::store::Store;

/// 恢复动作的事实名（`turn.stage{stage:"resume"}` 的 `result`）。
///
/// 与 `ResumeAction` 的 serde 线格式同值；`RetrySubtree` / `RetryCompaction`
/// 不落本格（前者按新轮处理，后者在进循环之前就返回了）。
pub(crate) fn resume_kind(action: &symbio_sdk::chat::chat_node::ResumeAction) -> &'static str {
    use symbio_sdk::chat::chat_node::ResumeAction;
    match action {
        ResumeAction::Retry => "retry",
        ResumeAction::Approve => "approve",
        ResumeAction::Reject => "reject",
        ResumeAction::Supply => "supply",
        ResumeAction::Answer => "answer",
        ResumeAction::RetrySubtree => "retry_subtree",
        ResumeAction::RetryCompaction => "retry_compaction",
    }
}

/// 落**派发档**（`turn.dispatch`）：本幕以哪一档派发、给多大预算、以谁的身份。
///
/// 幂等键带**幕号**（`d-{turn}-{drive_seq}`）：同一轮可以被恢复幕再执行一次，两幕的档位
/// 各记各的。
///
/// `budget_ms` 取 `tier.budget_ms()`，不取主体规格里那一格（`D-223` 订正 2）：规格的
/// 预算写在幕首建主体时，那一刻档位还没折出来，它只能记主体的**声明档**；本幕的实际
/// 预算归这一格。`I3`（`invariants::declared_budget_ms`）读的永远是这一格。
pub(crate) fn land_dispatch(
    store: &dyn Store<Event = Event>,
    turn_no: u64,
    user_seq: u64,
    drive_seq: u64,
    tier: symbio_kernel::adapters::LatencyTier,
    actor: &symbio_kernel::actors::ActorSpec,
) -> Result<(), symbio_kernel::store::AppendError> {
    store.append(
        Event::pending(
            format!("d-{turn_no}-{drive_seq}"),
            crate::control_decl::EVENT_TURN_DISPATCH,
            Entity::Turn,
            Verb::Asserted,
            turn_no,
            &actor.principal,
        )
        .with_produced_by(user_seq)
        // `budget_ms` 由**档位**折出，不由主体带（`D-223` 订正 2）：主体在幕首就建
        // （`D-222` 订正 3），那一刻档位还没定，规格里那一格只能是主体的**声明档**。
        // 本幕的预算因此只从这里出——`tier` 与 `budget_ms` 由同一个值派生，两格
        // 不可能对不上。
        .with_payload(serde_json::json!({
            "tier": tier.name(),
            "budget_ms": tier.budget_ms(),
            "principal": actor.principal,
        })),
    )?;
    Ok(())
}

/// 落**对话面开场白**（`chat.assistant.opening`，`Escalate` 首响）并让它上线。
///
/// 本轮随后**照进工具循环**，所以它落在 `turn × asserted`（不是收束格 N3「每轮至多一条」）。
/// 空正文不落格（与汇报 / 收束同一条纪律）。落格即上线：节点 id 用落格载荷里那个
/// （`dialog.id`），轮末同 id 再送一次时消费端按 id 合并成同一格。
///
/// `None` 的两样（没有首响 / 没有节点身份）都退化成「什么都不做」：两者必须同时在场，
/// 只有正文而没有节点身份会让实时面与重建面折出两个不同的节点。
pub(crate) fn land_opening(
    store: &dyn Store<Event = Event>,
    sink: &dyn symbio_sdk::adapters::DeltaSink,
    turn_no: u64,
    user_seq: u64,
    principal: &str,
    text: Option<&str>,
    dialog: Option<&super::super::turn_runner::DialogFacts>,
) -> Result<(), symbio_kernel::store::AppendError> {
    let (Some(opening), Some(dialog)) = (text, dialog) else {
        return Ok(());
    };
    if opening.trim().is_empty() {
        return Ok(());
    }
    store.append(
        Event::pending(
            format!("op-{turn_no}"),
            symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_OPENING,
            Entity::Turn,
            Verb::Asserted,
            turn_no,
            principal,
        )
        .with_produced_by(user_seq)
        .with_payload(serde_json::json!({
            "text": opening,
            "id": dialog.id,
            "reason": dialog.reason,
            "exclude_from_context": dialog.exclude_from_context,
        })),
    )?;
    let mut node = symbio_sdk::turn::node_build::dialog_node(
        opening,
        &dialog.reason,
        dialog.exclude_from_context,
        &symbio_sdk::chat::principal::Principal::from_legacy(principal),
    );
    node.id = dialog.id.clone();
    sink.on_tool_frame(&node);
    Ok(())
}
