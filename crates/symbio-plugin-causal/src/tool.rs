//! `causal_assert` —— 本插件唯一的 LLM 可见工具（名册 #39「因果表征工具」）。
//!
//! 它只做**入参校验与回执**：真正的落格不在这里，而在 [`super::behavior`] 交出的
//! `FactDraft` 与宿主唯一的写口（D-213）。这条分工让工具不持有事实源句柄，
//! 也让「这次调用产生了什么事实」由能力包自己声明，而不是由执行层按工具名猜。
//!
//! 工具**不声明 `tool_namespace`**：那是部署事实，声明它会让生成物写进
//! `crates/symbio-kernel/src/chat/tool_namespaces.gen.rs`——本条的验收判据正是
//! 「新增能力包 `kernel` 零 diff」。代价是工具主体地址落 `legacy/causal_assert`
//! （只影响转写里的展示身份，授权用的是调用方主体，见 D-176）。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest,
};

use super::behavior::CAUSAL_LINK_BEHAVIOR;
use super::decl::CAUSAL_TOOL;

pub(crate) struct CausalTool;

#[async_trait]
impl Capability for CausalTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: CAUSAL_TOOL.to_string(),
            description: "断言语义世界里的因果关系（「A 导致 B」）。断言之后，因果图会作为可读视图出现，后续判断可以引用它。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "cause": {
                        "type": "string",
                        "description": "原因（非空短句；与 effect 不同）"
                    },
                    "effect": {
                        "type": "string",
                        "description": "结果（非空短句；与 cause 不同）"
                    },
                    "confidence": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 100,
                        "description": "置信度 0–100，缺省 50。它是断言强度，不是概率计算的结果"
                    }
                },
                "required": ["cause", "effect"]
            }),
            category: Some(CapabilityCategory::Other),
            // 低风险：只写本插件自己的领域事实，不动工作目录、不发网络请求。
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                r#"{"cause":"内存不足","effect":"任务超时","confidence":70}"#.to_string()
            ]),
            behaviors: &[CAUSAL_LINK_BEHAVIOR],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (cause, effect, confidence) = extract(&args)?;
        Ok(json!({ "success": true, "cause": cause, "effect": effect, "confidence": confidence }))
    }
}

/// 入参的唯一校验点：两个非空短句、方向不能相同、置信度收进 0–100。
///
/// 抽成纯函数是为让判据不必构造执行环境。「因 == 果」直接拒而不是静默接受：
/// 自环在因果图里没有信息量，接受它只会让读方多一条要过滤的边。
fn extract(args: &Value) -> Result<(&str, &str, u64), PluginError> {
    let cause = args
        .get("cause")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("cause 必须是字符串".to_string()))?;
    let effect = args
        .get("effect")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("effect 必须是字符串".to_string()))?;
    if cause.is_empty() || effect.is_empty() {
        return Err(PluginError::ValidationError(
            "cause / effect 都不能为空".to_string(),
        ));
    }
    if cause == effect {
        return Err(PluginError::ValidationError(
            "cause 与 effect 不能相同（自环没有信息量）".to_string(),
        ));
    }
    let confidence = args
        .get("confidence")
        .map(|v| {
            v.as_u64().ok_or_else(|| {
                PluginError::ValidationError("confidence 必须是 0–100 的整数".to_string())
            })
        })
        .transpose()?
        .unwrap_or(50)
        .min(100);
    Ok((cause, effect, confidence))
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
