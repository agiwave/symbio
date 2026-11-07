//! `glossary_define` —— 本插件唯一的 LLM 可见工具。
//!
//! 它只做**入参校验与回执**：真正的落格在 [`super::behavior`] 交出的 `FactDraft`
//! 与宿主唯一的写口。工具不持有事实源句柄，也不按工具名写分支。
//!
//! 工具**不声明 `tool_namespace`**：那是部署事实，声明它会让生成物写进内核目录
//! （与 `notes` 夹具同一条边界，见 DE-025）。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest,
};

use super::behavior::GLOSSARY_RECORD;
use super::decl::{GLOSSARY_TOOL, MAX_DEFINITION_CHARS, MAX_TERM_CHARS};

pub(crate) struct GlossaryTool;

#[async_trait]
impl Capability for GlossaryTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: GLOSSARY_TOOL.to_string(),
            description: "定义一个项目术语（term + definition）；定义落进本会话的事实源，后续轮次的上下文会带上当前的术语表。同一个术语再次定义会覆盖旧释义。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "term": {
                        "type": "string",
                        "description": "术语名（非空，≤64 字符）"
                    },
                    "definition": {
                        "type": "string",
                        "description": "术语的准确释义（非空，≤500 字符）"
                    }
                },
                "required": ["term", "definition"]
            }),
            category: Some(CapabilityCategory::Other),
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                r#"{"term":"债","definition":"义务环里尚未闭合的一笔义务：必须由一条外部事实来清"}"#
                    .to_string(),
            ]),
            behaviors: &[GLOSSARY_RECORD],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (term, definition) = extract_pair(&args)?;
        Ok(json!({ "success": true, "term": term, "definition": definition }))
    }
}

/// 入参的唯一校验点：两个字段都必须是非空字符串，且长度在声明上界内。
fn extract_pair(args: &Value) -> Result<(&str, &str), PluginError> {
    Ok((
        extract_field(args, "term", MAX_TERM_CHARS)?,
        extract_field(args, "definition", MAX_DEFINITION_CHARS)?,
    ))
}

fn extract_field<'a>(
    args: &'a Value,
    field: &str,
    max_chars: usize,
) -> Result<&'a str, PluginError> {
    let raw = args
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| PluginError::ValidationError(format!("{field} 必须是字符串")))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(PluginError::ValidationError(format!("{field} 不能为空")));
    }
    if trimmed.chars().count() > max_chars {
        return Err(PluginError::ValidationError(format!(
            "{field} 不能超过 {max_chars} 个字符"
        )));
    }
    Ok(trimmed)
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
