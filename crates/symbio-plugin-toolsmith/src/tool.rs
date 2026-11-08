//! `toolsmith_define` —— 本插件唯一的 LLM 可见工具（名册 #41「制造工具（L2）」，受限）。
//!
//! 它只做**入参校验与回执**：真正的落格在 [`super::behavior`] 交出的 `FactDraft` 与
//! 宿主唯一的写口（D-213）。工具本身不持有事实源句柄。
//!
//! ## 为什么入参里没有 `owner`
//!
//! 「只能造自己插件的工具」靠**形状**落实：没有这个字段，调用方就没有地方填别人的插件；
//! 行为把 owner 写成常量 `toolsmith`，视图再按 owner 过滤一遍。想夹带
//! `{"owner": "vdfs"}` 只会被忽略——落格的仍是本插件自己的定义。
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

use super::behavior::TOOLSMITH_DEFINED_BEHAVIOR;
use super::decl::TOOLSMITH_TOOL;

pub(crate) struct ToolsmithDefineTool;

#[async_trait]
impl Capability for ToolsmithDefineTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: TOOLSMITH_TOOL.to_string(),
            description: "登记一个本插件自己的工具定义（名字、说明、入参 schema）。这是受限的制造工具：只能定义 toolsmith 自己的工具，且定义只进清单，进入可调用工具面要等下一次装配。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "工具名：小写字母开头，只含小写字母、数字与下划线，3–40 字符（如 summarize_ticket）"
                    },
                    "description": {
                        "type": "string",
                        "description": "这个工具做什么（非空短句）"
                    },
                    "input_schema": {
                        "type": "object",
                        "description": "这个工具的入参 JSON Schema（必须是对象）"
                    }
                },
                "required": ["name", "description", "input_schema"]
            }),
            category: Some(CapabilityCategory::Other),
            // 低风险：只写本插件自己的定义事实，不动工作目录、不发网络请求。
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                r#"{"name":"summarize_ticket","description":"把工单压成一句话","input_schema":{"type":"object","properties":{"ticket":{"type":"string"}},"required":["ticket"]}}"#.to_string(),
            ]),
            behaviors: &[TOOLSMITH_DEFINED_BEHAVIOR],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (name, description, schema) = extract(&args)?;
        Ok(json!({
            "success": true,
            "name": name,
            "description": description,
            "input_schema": schema,
            "owner": super::decl::OWNER_PLUGIN,
        }))
    }
}

/// 入参的唯一校验点：名字像工具名、说明非空、schema 是对象。
///
/// 抽成纯函数是为让判据不必构造执行环境。名字规则（小写字母开头、只含小写字母/数字/
/// 下划线）不是装饰：它是「这个名字将来能被工具面唯一解析」的最小前提。
fn extract(args: &Value) -> Result<(&str, &str, &Value), PluginError> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("name 必须是字符串".to_string()))?;
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("description 必须是字符串".to_string()))?;
    let schema = args
        .get("input_schema")
        .ok_or_else(|| PluginError::ValidationError("input_schema 必须是对象".to_string()))?;
    if !valid_name(name) {
        return Err(PluginError::ValidationError(
            "name 必须小写字母开头、只含小写字母/数字/下划线，且 3–40 字符".to_string(),
        ));
    }
    if description.is_empty() {
        return Err(PluginError::ValidationError(
            "description 不能为空".to_string(),
        ));
    }
    if !schema.is_object() {
        return Err(PluginError::ValidationError(
            "input_schema 必须是 JSON 对象".to_string(),
        ));
    }
    Ok((name, description, schema))
}

/// 工具名的最小形状：小写字母开头，3–40 字符，只含 `[a-z0-9_]`。
pub(crate) fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (3..=40).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
