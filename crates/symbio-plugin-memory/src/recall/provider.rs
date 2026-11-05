//! 召回段的上下文 provider（DE-025 D-208 / D-215 / D-220）。
//!
//! ## 为什么它在能力包里
//!
//! provider 渲染的是**检索读数**，而读数与它的排版口径都是检索能力的判据
//! （[`super::section`]）。段的位置与预算优先级是宿主的判断（`RECALL_PRIO`），
//! 但「这一段长什么样」不归控制面——否则每改一次措辞都要动控制面一次。
//!
//! 判定（哪些条目可见、哪些技能被摘掉）在宿主的轮首召回里已经做完：会话插件
//! 把**收放之后**的切片交给 `retrieve_all`，再把读数落成一格 `session.recall` 事实。
//! provider 拿不到会话存储（纯函数，DE-025 D-084），所以它从 [`ContextRequest::facts`]
//! 按名字读那一格——读的是**已判过的值**，无权改判，也不需要看见 `Retrieval` 结构本身。
//!
//! [`ContextRequest::recall`] 仍优先：宿主若已把读数直接递进来，就用递进来的值。

use symbio_plugin_api::context::{
    self, ContextProvider, ContextRequest, ContextSection, RecallFacts, RecallLine,
};
use symbio_plugin_api::events::EVENT_SESSION_RECALL;
use symbio_plugin_api::Event;

/// 段优先级（D-243）：预算放不下时先丢谁，数大者先留。
///
/// 召回是**背景事实**：丢了它，模型不知道自己此前做过什么，本轮每一步都要重新试错。
const RECALL_PRIO: u8 = 90;

pub(crate) struct RecallProvider;

impl ContextProvider for RecallProvider {
    fn owner(&self) -> &'static str {
        "memory"
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        let text = match req.recall {
            Some(facts) => super::section::prompt_section(facts),
            None => recall_from_facts(req.facts).and_then(|owned| {
                let lines: Vec<RecallLine<'_>> = owned
                    .lines
                    .iter()
                    .map(|line| RecallLine {
                        seq: line.seq,
                        content: line.content.as_str(),
                        tag: line.tag.as_str(),
                    })
                    .collect();
                super::section::prompt_section(&RecallFacts {
                    degraded: owned.degraded,
                    lines: &lines,
                })
            }),
        };
        match text {
            Some(text) => vec![ContextSection::new(
                context::slots::RECALL,
                text,
                RECALL_PRIO,
            )],
            None => Vec::new(),
        }
    }
}

/// 轮首那一格读数（`session.recall`）的形状：会话插件落格时的原样。
struct OwnedRecall {
    degraded: bool,
    lines: Vec<OwnedRecallLine>,
}

struct OwnedRecallLine {
    seq: u64,
    content: String,
    tag: String,
}

/// 从本会话事实里取**最近**一格召回读数（宿主每轮落一条，取最后一条）。
///
/// 空读数 = 本段缺席（不印空标题是既定设计）；`degraded` 由渲染方决定怎么出声。
fn recall_from_facts(events: &[Event]) -> Option<OwnedRecall> {
    let fact = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_SESSION_RECALL)?;
    let degraded = fact
        .payload
        .get("degraded")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let entries = fact.payload.get("entries")?.as_array()?;
    let lines: Vec<OwnedRecallLine> = entries
        .iter()
        .filter_map(|entry: &serde_json::Value| {
            Some(OwnedRecallLine {
                seq: entry.get("seq")?.as_u64()?,
                content: entry.get("content")?.as_str()?.to_string(),
                tag: entry.get("tag")?.as_str()?.to_string(),
            })
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(OwnedRecall {
        degraded,
        lines,
    })
}

symbio_plugin_api::submit_context_provider!(RecallProvider);

#[cfg(test)]
#[path = "provider.test.rs"]
mod tests;
