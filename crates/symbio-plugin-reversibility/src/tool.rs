//! `reversibility_annotate` —— 本插件唯一的 LLM 可见工具（名册 #40「可逆性设计」）。
//!
//! 它只做**入参校验与回执**：真正的落格在 [`super::behavior`] 交出的 `FactDraft` 与
//! 宿主唯一的写口（D-213）。工具本身不持有事实源句柄，也不判断动作到底能不能撤销——
//! 那是标注者的判断，工具只保证这条判断**说得具体**。
//!
//! 工具不声明 `tool_namespace`：那是部署事实，声明它会让生成物写进
//! `crates/symbio-kernel/src/chat/tool_namespaces.gen.rs`——本条的验收判据正是
//! 「新增能力包 `kernel` 零 diff」。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest,
};

use super::behavior::REVERSIBILITY_ASSESSED_BEHAVIOR;
use super::decl::{LEVELS, REVERSIBILITY_TOOL};

pub(crate) struct ReversibilityAnnotateTool;

#[async_trait]
impl Capability for ReversibilityAnnotateTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: REVERSIBILITY_TOOL.to_string(),
            description: "给一个动作标注可逆等级：reversible（能原路退回）、compensable（退不回但有补偿做法）、irreversible（退不回也没有补偿）。补偿做法在 compensable 时必须写清楚。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "被标注的动作（非空短句，如「删除 /tmp/build」「发送邮件给客户」）"
                    },
                    "level": {
                        "type": "string",
                        "enum": LEVELS,
                        "description": "可逆等级：reversible / compensable / irreversible"
                    },
                    "undo": {
                        "type": "string",
                        "description": "补偿做法；level=compensable 时必填且非空（如「把旧配置写回」）。其余等级可省略"
                    }
                },
                "required": ["action", "level"]
            }),
            category: Some(CapabilityCategory::Other),
            // 低风险：只写本插件自己的判断事实，不动工作目录、不发网络请求。
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                r#"{"action":"删除 /tmp/build","level":"reversible"}"#.to_string(),
                r#"{"action":"改配置","level":"compensable","undo":"把旧配置写回"}"#.to_string(),
            ]),
            behaviors: &[REVERSIBILITY_ASSESSED_BEHAVIOR],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (action, level, undo) = extract(&args)?;
        Ok(json!({
            "success": true,
            "action": action,
            "level": level,
            "undo": undo,
        }))
    }
}

/// 入参的唯一校验点：动作非空、等级在册、`compensable` 必须说得清补偿做法。
///
/// 抽成纯函数是为让判据不必构造执行环境。`compensable` 缺 `undo` 直接拒而不是降级成
/// `irreversible`：静默降级会把「标注者以为有后路」这件事藏起来，而拒会让它当场暴露。
fn extract(args: &Value) -> Result<(&str, &str, &str), PluginError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("action 必须是字符串".to_string()))?;
    let level = args
        .get("level")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("level 必须是字符串".to_string()))?;
    let undo = args
        .get("undo")
        .map(|v| {
            v.as_str()
                .map(str::trim)
                .ok_or_else(|| PluginError::ValidationError("undo 必须是字符串".to_string()))
        })
        .transpose()?
        .unwrap_or("");
    if action.is_empty() {
        return Err(PluginError::ValidationError("action 不能为空".to_string()));
    }
    if !LEVELS.contains(&level) {
        return Err(PluginError::ValidationError(format!(
            "level 必须是 {} 之一",
            LEVELS.join(" / ")
        )));
    }
    if level == "compensable" && undo.is_empty() {
        return Err(PluginError::ValidationError(
            "compensable 必须给出 undo（补偿做法）；没有补偿做法的可补偿是一次误判".to_string(),
        ));
    }
    Ok((action, level, undo))
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
