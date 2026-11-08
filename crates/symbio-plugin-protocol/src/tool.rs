//! `protocol_propose` / `protocol_join` —— 本插件的两个 LLM 可见工具（名册 #43）。
//!
//! 两者都只做**入参校验与回执**：真正的落格在 [`super::behavior`] 交出的 `FactDraft`
//! 与宿主唯一的写口（D-213）。工具本身不持有事实源句柄。
//!
//! 为什么是两个工具而不是一个：提案与加入是**不同的断言**（谁提的、谁加入），拆开后
//! 转写里各自成行，读方不需要从一条混合记录里猜「这半句是谁说的」。
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

use super::behavior::{PROTOCOL_JOINED_BEHAVIOR, PROTOCOL_PROPOSED_BEHAVIOR};
use super::decl::{PROTOCOL_JOIN_TOOL, PROTOCOL_PROPOSE_TOOL};

/// 提案工具：提出一个协议（名字 / 标题 / 目的 / 消息序列）。
pub(crate) struct ProtocolProposeTool;

#[async_trait]
impl Capability for ProtocolProposeTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: PROTOCOL_PROPOSE_TOOL.to_string(),
            description: "提出一个抽象协议：名字、标题、目的，以及消息序列（按发生顺序）。提出不等于成立——还需要另一方加入。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "协议名：小写字母开头，只含小写字母/数字/下划线，3–40 字符（如 handoff）"
                    },
                    "title": {
                        "type": "string",
                        "description": "协议标题（非空短句，如「任务交接」）"
                    },
                    "purpose": {
                        "type": "string",
                        "description": "这个协议解决什么问题（非空短句）"
                    },
                    "messages": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "消息序列：按发生顺序排列的非空消息名（如 request / accept / done）"
                    }
                },
                "required": ["name", "title", "purpose", "messages"]
            }),
            category: Some(CapabilityCategory::Other),
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                r#"{"name":"handoff","title":"任务交接","purpose":"把一个任务交给另一个代理","messages":["request","accept","done"]}"#.to_string(),
            ]),
            behaviors: &[PROTOCOL_PROPOSED_BEHAVIOR],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (name, title, purpose, messages) = extract_proposal(&args)?;
        Ok(json!({
            "success": true,
            "name": name,
            "title": title,
            "purpose": purpose,
            "messages": messages,
        }))
    }
}

/// 加入工具：第三方（或另一方）登记自己加入了某个协议。
pub(crate) struct ProtocolJoinTool;

#[async_trait]
impl Capability for ProtocolJoinTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: PROTOCOL_JOIN_TOOL.to_string(),
            description:
                "让一个参与者加入一个已提出的协议。不同参与者达到两个，协议才算成立（第二方原则）。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "协议名（必须是已提出的名字）"
                    },
                    "participant": {
                        "type": "string",
                        "description": "参与者标识（非空，最长 64 字符；如 main / reviewer）"
                    }
                },
                "required": ["name", "participant"]
            }),
            category: Some(CapabilityCategory::Other),
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                r#"{"name":"handoff","participant":"reviewer"}"#.to_string()
            ]),
            behaviors: &[PROTOCOL_JOINED_BEHAVIOR],
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let (name, participant) = extract_join(&args)?;
        Ok(json!({
            "success": true,
            "name": name,
            "participant": participant,
        }))
    }
}

/// 提案的校验点：名字像工具名、标题与目的非空、消息序列至少一条且都是非空短句。
fn extract_proposal(args: &Value) -> Result<(&str, &str, &str, Vec<String>), PluginError> {
    let text = |field: &str| -> Result<&str, PluginError> {
        args.get(field)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .ok_or_else(|| PluginError::ValidationError(format!("{field} 必须是字符串")))
    };
    let name = text("name")?;
    let title = text("title")?;
    let purpose = text("purpose")?;
    if !valid_name(name) {
        return Err(PluginError::ValidationError(
            "name 必须小写字母开头、只含小写字母/数字/下划线，且 3–40 字符".to_string(),
        ));
    }
    if title.is_empty() || purpose.is_empty() {
        return Err(PluginError::ValidationError(
            "title / purpose 都不能为空".to_string(),
        ));
    }
    let messages = messages_of(args)?;
    Ok((name, title, purpose, messages))
}

/// 加入的校验点：协议名合法、参与者非空且不超长。
fn extract_join(args: &Value) -> Result<(&str, &str), PluginError> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("name 必须是字符串".to_string()))?;
    let participant = args
        .get("participant")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| PluginError::ValidationError("participant 必须是字符串".to_string()))?;
    if !valid_name(name) {
        return Err(PluginError::ValidationError(
            "name 必须小写字母开头、只含小写字母/数字/下划线，且 3–40 字符".to_string(),
        ));
    }
    if participant.is_empty() || participant.chars().count() > 64 {
        return Err(PluginError::ValidationError(
            "participant 不能为空，且不超过 64 字符".to_string(),
        ));
    }
    Ok((name, participant))
}

/// 消息序列：非空字符串数组，去空白、按首次出现去重，至少一条。
pub(super) fn messages_of(payload: &Value) -> Result<Vec<String>, PluginError> {
    let raw = payload
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| PluginError::ValidationError("messages 必须是字符串数组".to_string()))?;
    let mut out: Vec<String> = Vec::new();
    for item in raw {
        let m = item
            .as_str()
            .map(str::trim)
            .ok_or_else(|| PluginError::ValidationError("messages 里只能有字符串".to_string()))?;
        if m.is_empty() {
            return Err(PluginError::ValidationError(
                "messages 里不能有空消息".to_string(),
            ));
        }
        if !out.iter().any(|x| x == m) {
            out.push(m.to_string());
        }
    }
    if out.is_empty() {
        return Err(PluginError::ValidationError(
            "messages 至少要有一条".to_string(),
        ));
    }
    Ok(out)
}

/// 名字的最小形状：小写字母开头，3–40 字符，只含 `[a-z0-9_]`。
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
