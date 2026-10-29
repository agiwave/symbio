//! `notes_add` —— 本插件唯一的 LLM 可见工具。
//!
//! 它只做**入参校验与回执**：真正的落格不在这里，而在 [`super::behavior`] 交出的
//! `FactDraft` 与宿主轮末的唯一写口（D-213）。这条分工让工具不持有事实源句柄，
//! 也让「这次调用产生了什么事实」由能力包自己声明，而不是由执行层按工具名猜。
//!
//! 工具**不声明 `tool_namespace`**：那是部署事实，声明它会让生成物写进
//! `crates/symbio-kernel/src/chat/tool_namespaces.gen.rs`——本夹具的验收判据正是
//! 「新增能力包 `kernel` 零 diff」。代价是工具主体地址落 `legacy/notes_add`
//! （只影响转写里的展示身份，授权用的是调用方主体，见 D-176）。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_core::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest,
};

use super::behavior::NOTE_CAPTURE;
use super::decl::NOTES_TOOL;

pub(crate) struct NotesTool;

#[async_trait]
impl Capability for NotesTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: NOTES_TOOL.to_string(),
            description: "把一条便签记进本会话的事实源；成功之后，最近的几条便签会作为上下文出现在后续对话里。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "便签正文（非空）"
                    }
                },
                "required": ["text"]
            }),
            category: Some(CapabilityCategory::Other),
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![r#"{"text":"买牛奶"}"#.to_string()]),
            behaviors: &[NOTE_CAPTURE],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let text = extract_text(&args)?;
        Ok(json!({ "success": true, "text": text }))
    }
}

/// 入参的唯一校验点：`text` 必须是非空字符串。抽出成纯函数是为让判据不必构造执行环境。
fn extract_text(args: &Value) -> Result<&str, PluginError> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PluginError::ValidationError("text 必须是字符串".to_string()))?;
    let text = text.trim();
    if text.is_empty() {
        return Err(PluginError::ValidationError("text 不能为空".to_string()));
    }
    Ok(text)
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
