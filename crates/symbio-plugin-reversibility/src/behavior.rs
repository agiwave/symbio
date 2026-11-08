//! 效果行为：一次成功的 `reversibility_annotate` ⇒ 一条 `reversibility.assessed` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：唯一的写口（`loop` 的 `record_derived`）
//! 补上宿主面再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**相连。
//!
//! 幂等键用 `EffectCall::anchor` + 调用 id + 被标注的动作：同一次重放不会记两次，
//! 而跨请求对同一动作的**修正判断**会各自成键（登记表取最后一条）。
//!
//! ## 为什么没有 `before`
//!
//! 可逆等级是判断，不是事实的累加：后来的判断可以推翻先前的判断。把「同一动作再次
//! 标注」当成重复挡掉，会让升级判断（发现目录里有客户数据）无法落格。去重只在幂等键
//! 那一层（同一次调用重放）。

use symbio_plugin_api::behaviors::Kind;
use symbio_plugin_api::{EffectCall, FactDraft, ToolBehavior};
use symbio_plugin_api::{Entity, Verb};

use super::decl::{LEVELS, REVERSIBILITY_ASSESSED};

/// 本行为管辖的名字。**由本 crate 自报**（D-363）：工具在自己的
/// `CapabilityMeta.behaviors` 里声明同一个常量，宿主只做收集。
pub(crate) const REVERSIBILITY_ASSESSED_BEHAVIOR: &str = "reversibility_assessed";

/// 参数里的标注三元组：动作 / 等级 / 补偿做法。写方与工具回执共用这一口径。
fn assessment(value: &serde_json::Value) -> Option<(&str, &str, &str)> {
    let action = value.get("action")?.as_str()?.trim();
    let level = value.get("level")?.as_str()?.trim();
    let undo = value
        .get("undo")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if action.is_empty() || !LEVELS.contains(&level) {
        return None;
    }
    if level == "compensable" && undo.is_empty() {
        return None;
    }
    Some((action, level, undo))
}

/// 一次成功的调用 = 一条可逆性判断。
pub(crate) struct ReversibilityCapture;

impl ToolBehavior for ReversibilityCapture {
    fn owner(&self) -> &'static str {
        "reversibility"
    }

    fn behavior(&self) -> &'static str {
        REVERSIBILITY_ASSESSED_BEHAVIOR
    }

    fn kind(&self) -> Kind {
        Kind::LandsFacts
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some((action, level, undo)) = assessment(call.args) else {
            return Vec::new();
        };
        vec![FactDraft {
            // 幂等键带上被标注的动作：同一次调用重放落同一条，修正判断各自成立
            key: format!("r-{}-{}-{action}", call.anchor, call.call_id),
            kind: REVERSIBILITY_ASSESSED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "action": action,
                "level": level,
                "undo": undo,
            }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按声明选它。
symbio_plugin_api::submit_behavior!(ReversibilityCapture);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
