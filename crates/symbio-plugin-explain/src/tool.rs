//! `explain_record` —— 本插件唯一的 LLM 可见工具（名册 #49「可解释」）。
//!
//! 它只做**入参校验与回执**：真正的落格不在这里，而在 [`super::behavior`] 交出的
//! `FactDraft` 与宿主唯一的写口（D-213）。这条分工让工具不持有事实源句柄，
//! 也让「这次调用产生了什么事实」由能力包自己声明，而不是由执行层按工具名猜。
//!
//! 工具**不声明 `tool_namespace`**：那是部署事实，声明它会让生成物写进
//! `crates/symbio-kernel/src/chat/tool_namespaces.gen.rs`——本条的验收判据正是
//! 「新增能力包 `kernel` 零 diff」。代价是工具主体地址落 `legacy/explain_record`
//! （只影响转写里的展示身份，授权用的是调用方主体，见 D-176）。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest,
};

use super::behavior::EXPLAIN_RATIONALE_BEHAVIOR;
use super::decl::EXPLAIN_TOOL;

pub(crate) struct ExplainTool;

#[async_trait]
impl Capability for ExplainTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: EXPLAIN_TOOL.to_string(),
            description: "为一次决定或动作记下「为什么」。记录之后，理由会作为可读台账出现，后续判断可以引用它。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "decision": {
                        "type": "string",
                        "description": "决定的动作或结论（非空短句）"
                    },
                    "why": {
                        "type": "string",
                        "description": "这样决定的理由（非空短句）"
                    },
                    "evidence": {
                        "type": "string",
                        "description": "可选的证据 / 观测；缺省为空"
                    }
                },
                "required": ["decision", "why"]
            }),
            category: Some(CapabilityCategory::Other),
            // 低风险：只写本插件自己的领域事实，不动工作目录、不发网络请求。
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                r#"{"decision":"切换到增量编译","why":"全量编译超过预算","evidence":"构建日志"}"#
                    .to_string(),
            ]),
            behaviors: &[EXPLAIN_RATIONALE_BEHAVIOR],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (decision, why, evidence) = extract(&args)?;
        Ok(json!({
            "success": true,
            "decision": decision,
            "why": why,
            "evidence": evidence,
        }))
    }
}

/// 入参的唯一校验点：两个非空短句 + 一个可选字符串。
///
/// 抽成纯函数是为让判据不必构造执行环境。`evidence` 缺省空串而不是拒绝：它是可选
/// 观测，不是每条理由都必须有；但**填错类型**要拒——「没填」与「填错了」是两件事。
fn extract(args: &Value) -> Result<(&str, &str, &str), PluginError> {
    let decision = args
        .get("decision")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("decision 必须是字符串".to_string()))?;
    let why = args
        .get("why")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("why 必须是字符串".to_string()))?;
    if decision.is_empty() || why.is_empty() {
        return Err(PluginError::ValidationError(
            "decision / why 都不能为空".to_string(),
        ));
    }
    let evidence = match args.get("evidence") {
        None => "",
        Some(Value::String(s)) => s.trim(),
        Some(_) => {
            return Err(PluginError::ValidationError(
                "evidence 必须是字符串（可选）".to_string(),
            ))
        }
    };
    Ok((decision, why, evidence))
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
