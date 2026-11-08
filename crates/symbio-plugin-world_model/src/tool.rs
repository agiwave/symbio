//! `world_observe` —— 本插件唯一的 LLM 可见工具（名册 #35「世界模型」）。
//!
//! 它只做**入参校验与回执**：真正的落格不在这里，而在 [`super::behavior`] 交出的
//! `FactDraft` 与宿主唯一的写口（D-213）。这条分工让工具不持有事实源句柄，
//! 也让「这次调用产生了什么事实」由能力包自己声明，而不是由执行层按工具名猜。
//!
//! 工具**不声明 `tool_namespace`**：那是部署事实，声明它会让生成物写进
//! `crates/symbio-kernel/src/chat/tool_namespaces.gen.rs`——本条的验收判据正是
//! 「新增能力包 `kernel` 零 diff」。代价是工具主体地址落 `legacy/world_observe`
//! （只影响转写里的展示身份，授权用的是调用方主体，见 D-176）。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest,
};

use super::behavior::WORLD_OBSERVED_BEHAVIOR;
use super::decl::WORLD_TOOL;

pub(crate) struct WorldObserveTool;

#[async_trait]
impl Capability for WorldObserveTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: WORLD_TOOL.to_string(),
            description: "记下一条世界状态观察：某个实体的某个属性当前是什么值。观察会累积成可读的世界模型视图，后续判断可以引用它。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "entity": {
                        "type": "string",
                        "description": "被观察的实体（非空短句，如「服务器」「数据库」）"
                    },
                    "attribute": {
                        "type": "string",
                        "description": "实体的属性名（非空短句，如「状态」「版本」）"
                    },
                    "value": {
                        "type": "string",
                        "description": "这次观察到的值（非空；同一实体同一属性后写覆盖）"
                    },
                    "confidence": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 100,
                        "description": "置信度 0–100，缺省 50。它是观察强度，不是概率计算的结果"
                    }
                },
                "required": ["entity", "attribute", "value"]
            }),
            category: Some(CapabilityCategory::Other),
            // 低风险：只写本插件自己的领域事实，不动工作目录、不发网络请求。
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                r#"{"entity":"服务器","attribute":"状态","value":"离线","confidence":80}"#.to_string()
            ]),
            behaviors: &[WORLD_OBSERVED_BEHAVIOR],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (entity, attribute, value, confidence) = extract(&args)?;
        Ok(json!({
            "success": true,
            "entity": entity,
            "attribute": attribute,
            "value": value,
            "confidence": confidence,
        }))
    }
}

/// 入参的唯一校验点：三个非空短句、置信度收进 0–100。
///
/// 抽成纯函数是为让判据不必构造执行环境。空值直接拒而不是静默接受：一条「属性等于
/// 空串」的观察在读数里没有信息量，接受它只会让读方多一条要过滤的格子。
fn extract(args: &Value) -> Result<(&str, &str, &str, u64), PluginError> {
    let field = |name: &str| -> Result<&str, PluginError> {
        args.get(name)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .ok_or_else(|| PluginError::ValidationError(format!("{name} 必须是字符串")))
    };
    let entity = field("entity")?;
    let attribute = field("attribute")?;
    let value = field("value")?;
    if entity.is_empty() || attribute.is_empty() || value.is_empty() {
        return Err(PluginError::ValidationError(
            "entity / attribute / value 都不能为空".to_string(),
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
    Ok((entity, attribute, value, confidence))
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
