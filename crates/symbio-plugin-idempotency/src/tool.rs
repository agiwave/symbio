//! `idempotent_record` —— 本插件唯一的 LLM 可见工具（名册 #37「幂等与重试」的幂等半边）。
//!
//! 它只做**入参校验与回执**：真正的落格不在这里，而在 [`super::behavior`] 交出的
//! `FactDraft` 与宿主唯一的写口（D-213）。这条分工让工具不持有事实源句柄，
//! 也让「这次调用产生了什么事实」由能力包自己声明，而不是由执行层按工具名猜。
//!
//! 工具**不声明 `tool_namespace`**：那是部署事实，声明它会让生成物写进
//! `crates/symbio-kernel/src/chat/tool_namespaces.gen.rs`——本条的验收判据正是
//! 「新增能力包 `kernel` 零 diff」。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest,
};

use super::behavior::IDEMPOTENCY_RECORD_BEHAVIOR;
use super::decl::IDEMPOTENCY_TOOL;

pub(crate) struct IdempotentRecordTool;

#[async_trait]
impl Capability for IdempotentRecordTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: IDEMPOTENCY_TOOL.to_string(),
            description: "把一个键与它的值记进幂等台账。同一个键在同一会话里只记一次；重复调用会在执行前被拒，模型看到的是「已经记过」。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "幂等键（非空；同一会话内唯一）"
                    },
                    "value": {
                        "type": "string",
                        "description": "这个键对应的值（非空）"
                    }
                },
                "required": ["key", "value"]
            }),
            category: Some(CapabilityCategory::Other),
            // 低风险：只写本插件自己的领域事实，不动工作目录、不发网络请求。
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![r#"{"key":"build-42","value":"compiled"}"#.to_string()]),
            behaviors: &[IDEMPOTENCY_RECORD_BEHAVIOR],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (key, value) = extract(&args)?;
        Ok(json!({
            "success": true,
            "key": key,
            "value": value,
        }))
    }
}

/// 入参的唯一校验点：两个非空短句。抽成纯函数是为让判据不必构造执行环境。
fn extract(args: &Value) -> Result<(&str, &str), PluginError> {
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("key 必须是字符串".to_string()))?;
    let value = args
        .get("value")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("value 必须是字符串".to_string()))?;
    if key.is_empty() || value.is_empty() {
        return Err(PluginError::ValidationError(
            "key / value 都不能为空".to_string(),
        ));
    }
    Ok((key, value))
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
