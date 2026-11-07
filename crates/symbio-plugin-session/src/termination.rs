//! 会话的终止写方（D-367）：一个 scope 算完的判据在这里，不在环里。
//!
//! 判据两件：**已落终答**（`chat.assistant.final`）∧ **宿主报告无未还债**
//! （`TerminationView::exhausted`，即本次驱动 `NoDebt`）。写方只看投影，不读环的
//! 内部状态；落 `session.terminated` 之后，这一格就成为普通世界事实——环下一圈
//! 经同一投影读回（需要它的债行用 `unless` 按名字问）。
//!
//! 终止格本身是 `Scope × Closed`，属于 I3 的「这一轮为什么停」判定范围：`reason`
//! **原样取终答事实上的那一格**，不在这里另起一个值——「为什么停」只有一个 owner
//! （写终答的那一行），抄一份就会有两处会漂的答案。终答没带 `reason` 时本写方不落格：
//! 那是终答自身的 I3 缺口，用一个编造的值盖住它才是更坏的失败。
//!
//! 与 `session.settled`（`close_turn`）的分工：`settled` 是**收束贡献**的关账
//! （终答之后跑行、落贡献），`terminated` 是**整轮**的关账（终答 ∧ 无债）。
//! 两者条件不同，不能互相替代。

use std::sync::Arc;

use symbio_kernel::capability::CapabilityRow;
use symbio_kernel::event::{Entity, Verb};
use symbio_plugin_api::drive::DriveFact;
use symbio_plugin_api::termination::{termination_row, TerminationView, TerminationWriter};
use symbio_plugin_api::FactDraft;

use crate::declarations::EVENT_SESSION_TERMINATED;

/// 终答事实名（由 `loop` 的词汇表拥有；这里只按名字读）。
const EVENT_FINAL: &str = symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL;

/// 会话的簿记写方：谓词成立就落 `session.terminated`。
pub(crate) struct SessionTermination;

impl TerminationWriter for SessionTermination {
    fn observe(&self, view: &TerminationView<'_>) -> Vec<FactDraft> {
        // 「无未还债」是宿主的结论（`NoDebt`），写方不自行判环。
        if !view.exhausted {
            return Vec::new();
        }
        let Some((root_id, reason)) = latest_final(view.facts) else {
            return Vec::new();
        };
        if already_terminated(view.facts, root_id) {
            return Vec::new();
        }
        vec![FactDraft {
            key: format!("{EVENT_SESSION_TERMINATED}:{root_id}"),
            kind: EVENT_SESSION_TERMINATED.to_string(),
            entity: Entity::Scope,
            verb: Verb::Closed,
            payload: serde_json::json!({ "root_id": root_id, "reason": reason }),
            // 溯源由写口补（本会话最近一条事实）——与通用世界写口同一条口径。
            produced_by: None,
        }]
    }
}

/// 本插件注册的 `termination` 行（宿主收集，不认插件名）。
pub(crate) fn rows() -> Vec<CapabilityRow> {
    vec![termination_row(
        "session",
        Arc::new(SessionTermination) as Arc<dyn TerminationWriter>,
    )]
}

/// 最近一条终答的 `(root_id, reason)`；两格缺一都不成立。
fn latest_final(facts: &[DriveFact]) -> Option<(&str, &str)> {
    facts
        .iter()
        .rev()
        .filter(|fact| fact.kind == EVENT_FINAL)
        .find_map(|fact| {
            let root_id = fact.payload.get("root_id").and_then(|v| v.as_str())?;
            let reason = fact.payload.get("reason").and_then(|v| v.as_str())?;
            Some((root_id, reason))
        })
}

fn already_terminated(facts: &[DriveFact], root_id: &str) -> bool {
    facts.iter().any(|fact| {
        fact.kind == EVENT_SESSION_TERMINATED
            && fact.payload.get("root_id").and_then(|v| v.as_str()) == Some(root_id)
    })
}

#[cfg(test)]
#[path = "termination.test.rs"]
mod tests;
