//! 效果行为：一次成功的 `explain_record` ⇒ 一条 `explain.rationale` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：唯一的写口（`loop` 的 `record_derived`）
//! 补上宿主面再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**
//! 相连——宿主两侧都不认识 `explain_record` 这个工具名。
//!
//! 幂等键用 `EffectCall::anchor`（宿主给的不透明词干）+ 调用 id + 决定的短句：同一次
//! 重放不会把一条理由记两次，而跨请求同名调用也不会互相吞掉。
//!
//! ## 判据在入参上，不在事实里
//!
//! 「同一个决定 + 同一条理由重复记录」是**入参**就能看出来的问题，不需要扫全量事实。
//! 同一个决定换一条理由不是重复——那是同一决定的新理由，是一条新信息。

use symbio_plugin_api::{
    behaviors, BeforeDecision, BeforeRefusal, BeforeRequest, EffectCall, FactDraft, ToolBehavior,
};
use symbio_plugin_api::{Entity, Verb};

use super::decl::EXPLAIN_RATIONALE;

/// 本行为管辖的名字（宿主词汇，见 `symbio_plugin_api::behaviors`）。
pub(crate) const EXPLAIN_RATIONALE_BEHAVIOR: &str = behaviors::EXPLAIN_RATIONALE;

/// 参数里的三元组：决定 / 理由 / 证据。三处（判方、写方、工具回执）共用这一个口径。
fn triple(value: &serde_json::Value) -> Option<(&str, &str, &str)> {
    let decision = value.get("decision")?.as_str()?.trim();
    let why = value.get("why")?.as_str()?.trim();
    if decision.is_empty() || why.is_empty() {
        return None;
    }
    // 证据缺省为空串；填了但不是字符串 ⇒ 没有效果（工具侧会先拒，这里不依赖它兜底）。
    let evidence = match value.get("evidence") {
        None => "",
        Some(serde_json::Value::String(s)) => s.trim(),
        Some(_) => return None,
    };
    Some((decision, why, evidence))
}

/// 一次成功的调用 = 一条理由记录。
pub(crate) struct ExplainCapture;

impl ToolBehavior for ExplainCapture {
    fn owner(&self) -> &'static str {
        "explain"
    }

    fn behavior(&self) -> &'static str {
        EXPLAIN_RATIONALE_BEHAVIOR
    }

    /// 同一会话里已经记录过**同一个决定 + 同一条理由** ⇒ 拒绝这次写入。
    ///
    /// 判据只看 `req.facts` 里的 `explain.rationale`（本插件自己的事件名）：既不需要
    /// 知道工具名，也不需要任何跨插件约定。
    fn before(&self, req: &BeforeRequest<'_>) -> BeforeDecision {
        let Some((decision, why, _)) = triple(req.args) else {
            return BeforeDecision::Allow;
        };
        let dup = req.facts.iter().any(|e| {
            e.kind == EXPLAIN_RATIONALE
                && e.payload.get("decision").and_then(|v| v.as_str()) == Some(decision)
                && e.payload.get("why").and_then(|v| v.as_str()) == Some(why)
        });
        if !dup {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(BeforeRefusal::new(
            "理由记录",
            format!("这条理由已经记录过（同一会话内同一决定 + 同一理由）：{decision} — {why}"),
            symbio_plugin_api::failure_kind::ERROR,
        ))
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some((decision, why, evidence)) = triple(call.args) else {
            return Vec::new();
        };
        vec![FactDraft {
            // 幂等键带上决定的短句：同一决定的不同调用不能互相吞掉
            key: format!("x-{}-{}-{decision}", call.anchor, call.call_id),
            kind: EXPLAIN_RATIONALE.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "decision": decision,
                "why": why,
                "evidence": evidence,
            }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按声明选它。
symbio_plugin_api::submit_behavior!(ExplainCapture);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
