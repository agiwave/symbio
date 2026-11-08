//! 效果行为：一次成功的 `toolsmith_define` ⇒ 一条 `toolsmith.defined` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：唯一的写口（`loop` 的 `record_derived`）
//! 补上宿主面再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**相连。
//!
//! 幂等键用 `EffectCall::anchor` + 调用 id + 工具名：同一次重放不会记两次，而跨请求的
//! **定义修订**会各自成键（清单取最后一条）。
//!
//! ## owner 是常量，不是入参
//!
//! 「只能造自己插件的工具」在这里第二次落地：即使工具侧忘了校验、或调用方在 args 里
//! 夹带了 `owner`，草稿里的 owner 也只可能是 `toolsmith`。
//!
//! ## 为什么没有 `before`
//!
//! 同名再登记按后写覆盖（描述或 schema 可以不同），不是错误。去重只在幂等键那一层
//! （同一次调用重放）。

use symbio_plugin_api::behaviors::Kind;
use symbio_plugin_api::{EffectCall, FactDraft, ToolBehavior};
use symbio_plugin_api::{Entity, Verb};

use super::decl::{OWNER_PLUGIN, TOOLSMITH_DEFINED};
use super::tool::valid_name;

/// 本行为管辖的名字。**由本 crate 自报**（D-363）：工具在自己的
/// `CapabilityMeta.behaviors` 里声明同一个常量，宿主只做收集。
pub(crate) const TOOLSMITH_DEFINED_BEHAVIOR: &str = "toolsmith_defined";

/// 参数里的定义三元组：名字 / 说明 / schema。写方与工具回执共用这一口径。
fn definition(value: &serde_json::Value) -> Option<(&str, &str, &serde_json::Value)> {
    let name = value.get("name")?.as_str()?.trim();
    let description = value.get("description")?.as_str()?.trim();
    let schema = value.get("input_schema")?;
    if !valid_name(name) || description.is_empty() || !schema.is_object() {
        return None;
    }
    Some((name, description, schema))
}

/// 一次成功的调用 = 一条本插件自己的工具定义。
pub(crate) struct ToolsmithCapture;

impl ToolBehavior for ToolsmithCapture {
    fn owner(&self) -> &'static str {
        "toolsmith"
    }

    fn behavior(&self) -> &'static str {
        TOOLSMITH_DEFINED_BEHAVIOR
    }

    fn kind(&self) -> Kind {
        Kind::LandsFacts
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some((name, description, schema)) = definition(call.args) else {
            return Vec::new();
        };
        vec![FactDraft {
            // 幂等键带上工具名：同一次调用重放落同一条，定义修订各自成立
            key: format!("td-{}-{}-{name}", call.anchor, call.call_id),
            kind: TOOLSMITH_DEFINED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "name": name,
                "description": description,
                "input_schema": schema,
                // 常量：不听调用方的
                "owner": OWNER_PLUGIN,
            }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按声明选它。
symbio_plugin_api::submit_behavior!(ToolsmithCapture);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
