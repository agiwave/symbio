//! 效果行为：一次成功的 `causal_assert` ⇒ 一条 `causal.link` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：唯一的写口（`loop` 的 `record_derived`）
//! 补上宿主面再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**
//! 相连——宿主两侧都不认识 `causal_assert` 这个工具名。
//!
//! 幂等键用 `EffectCall::anchor`（宿主给的不透明词干）+ 调用 id：同一次重放不会把
//! 一条断言记两次，而跨请求同名调用也不会互相吞掉。
//!
//! ## 判据在入参上，不在事实里
//!
//! 「因」与「果」重复、或 A→B 与 B→A 同时成立，都是**入参**就能看出来的问题，
//! 不需要扫全量事实。它们不是拒绝（用户可能真的要说「互为因果」），而是
//! 收窄到一条可用的断言：同一条断言重复断言只留一条（靠幂等键），方向相反的
//! 两条各自成立——后者由读方（视图的 `edges`）自己看见，本层不替它裁决。

use symbio_plugin_api::{
    behaviors, BeforeDecision, BeforeRefusal, BeforeRequest, EffectCall, FactDraft, ToolBehavior,
};
use symbio_plugin_api::{Entity, Verb};

use super::decl::CAUSAL_LINK;

/// 本行为管辖的名字（宿主词汇，见 `symbio_plugin_api::behaviors`）。
pub(crate) const CAUSAL_LINK_BEHAVIOR: &str = behaviors::CAUSAL_LINK;

/// 参数里的三元组：因 / 果 / 置信度。三处（判方、写方、工具回执）共用这一个口径。
fn triple(value: &serde_json::Value) -> Option<(&str, &str, u64)> {
    let cause = value.get("cause")?.as_str()?.trim();
    let effect = value.get("effect")?.as_str()?.trim();
    if cause.is_empty() || effect.is_empty() || cause == effect {
        return None;
    }
    let confidence = value
        .get("confidence")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(100);
    Some((cause, effect, confidence))
}

/// 一次成功的调用 = 一条因果断言。
pub(crate) struct CausalCapture;

impl ToolBehavior for CausalCapture {
    fn owner(&self) -> &'static str {
        "causal"
    }

    fn behavior(&self) -> &'static str {
        CAUSAL_LINK_BEHAVIOR
    }

    /// 同一会话里已经断言过**同一条**（同向、同因、同果）⇒ 拒绝这次写入。
    ///
    /// 判据只看 `req.facts` 里的 `causal.link`（本插件自己的事件名）：既不需要知道
    /// 工具名，也不需要任何跨插件约定。方向相反的断言不落在这里——它是一条新信息。
    fn before(&self, req: &BeforeRequest<'_>) -> BeforeDecision {
        let Some((cause, effect, _)) = triple(req.args) else {
            return BeforeDecision::Allow;
        };
        let dup = req.facts.iter().any(|e| {
            e.kind == CAUSAL_LINK
                && e.payload.get("cause").and_then(|v| v.as_str()) == Some(cause)
                && e.payload.get("effect").and_then(|v| v.as_str()) == Some(effect)
        });
        if !dup {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(BeforeRefusal::new(
            "因果断言",
            format!("这条因果关系已经断言过（同一会话内同向重复）：{cause} → {effect}"),
            symbio_plugin_api::failure_kind::ERROR,
        ))
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some((cause, effect, confidence)) = triple(call.args) else {
            return Vec::new();
        };
        vec![FactDraft {
            // 幂等键带上方向：A→B 与 B→A 是两条断言，不能互相吞掉
            key: format!("c-{}-{}-{cause}->{effect}", call.anchor, call.call_id),
            kind: CAUSAL_LINK.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "cause": cause,
                "effect": effect,
                "confidence": confidence,
            }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按声明选它。
symbio_plugin_api::submit_behavior!(CausalCapture);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
