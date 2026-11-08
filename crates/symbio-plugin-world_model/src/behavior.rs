//! 效果行为：一次成功的 `world_observe` ⇒ 一条 `world.observed` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：唯一的写口（`loop` 的 `record_derived`）
//! 补上宿主面再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**
//! 相连——宿主两侧都不认识 `world_observe` 这个工具名。
//!
//! 幂等键用 `EffectCall::anchor`（宿主给的不透明词干）+ 调用 id + 被观察的键：
//! 同一次重放不会把一条观察记两次，而跨请求同名调用也不会互相吞掉。
//!
//! ## 为什么没有 `before`
//!
//! 世界模型的价值在**变化**：同一实体同一属性被再次观察，是「它变了」的证据，不是
//! 重复。去重只在幂等键那一层（同一次调用重放），不把「后来的观察」当成错误挡掉。

use symbio_plugin_api::behaviors::Kind;
use symbio_plugin_api::{EffectCall, FactDraft, ToolBehavior};
use symbio_plugin_api::{Entity, Verb};

use super::decl::WORLD_OBSERVED;

/// 本行为管辖的名字。**由本 crate 自报**（D-363）：工具在自己的
/// `CapabilityMeta.behaviors` 里声明同一个常量，宿主只做收集。
pub(crate) const WORLD_OBSERVED_BEHAVIOR: &str = "world_observed";

/// 参数里的观察四元组：实体 / 属性 / 值 / 置信度。两处（写方、工具回执）共用这一口径。
fn observation(value: &serde_json::Value) -> Option<(&str, &str, &str, u64)> {
    let entity = value.get("entity")?.as_str()?.trim();
    let attribute = value.get("attribute")?.as_str()?.trim();
    let observed = value.get("value")?.as_str()?.trim();
    if entity.is_empty() || attribute.is_empty() || observed.is_empty() {
        return None;
    }
    let confidence = value
        .get("confidence")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(100);
    Some((entity, attribute, observed, confidence))
}

/// 一次成功的调用 = 一条世界状态观察。
pub(crate) struct WorldCapture;

impl ToolBehavior for WorldCapture {
    fn owner(&self) -> &'static str {
        "world_model"
    }

    fn behavior(&self) -> &'static str {
        WORLD_OBSERVED_BEHAVIOR
    }

    fn kind(&self) -> Kind {
        Kind::LandsFacts
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some((entity, attribute, value, confidence)) = observation(call.args) else {
            return Vec::new();
        };
        vec![FactDraft {
            // 幂等键带上「被观察的键」：同一次调用重放落同一条，不同观察各自成立
            key: format!("w-{}-{}-{entity}.{attribute}", call.anchor, call.call_id),
            kind: WORLD_OBSERVED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "entity": entity,
                "attribute": attribute,
                "value": value,
                "confidence": confidence,
            }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按声明选它。
symbio_plugin_api::submit_behavior!(WorldCapture);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
