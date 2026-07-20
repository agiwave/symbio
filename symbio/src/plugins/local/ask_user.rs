//! 询问用户工具 - 实现 Capability（对应 Trae 的 AskUserQuestion）
//!
//! 支持单问题（question）或批量问题（questions[]，1~4 个，对齐 Trae）。
//! 后端通过 `Session` 通道广播一个 `user_prompt` 消息节点（status = WaitingUserAction），
//! 编排层在本轮结束时将会话置于 `AwaitingInput(user)`；用户答案以一条普通 `user` 消息
//! （`meta.responds_to` 指向本节点）回填后，新一轮会重跑本工具并拿到答案。
//! options 自动补充 "Other" 选项。详见 USER_INPUT_MECHANISM 设计文档。

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    schemas::session::chat_message::{
        ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
    },
    schemas::session::session_chat_response,
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginChannel,
    PluginFrame, PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_QUESTIONS: usize = 4;
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 4;

#[derive(Clone)]
pub struct AskUserTool {
    #[allow(dead_code)]
    security: Arc<SecurityPolicy>,
}
impl AskUserTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    /// 构造 `user_prompt` 节点载荷（含问题或确认信息）
    fn build_prompt_payload(&self, args: &Value) -> Result<Value, String> {
        // 批量模式：questions[]
        if let Some(questions) = args.get("questions").and_then(|v| v.as_array()) {
            if questions.is_empty() || questions.len() > MAX_QUESTIONS {
                return Err(format!("questions 数量需在 1~{MAX_QUESTIONS} 之间"));
            }
            let mut normalized = Vec::with_capacity(questions.len());
            for q in questions {
                normalized.push(self.normalize_question(q)?);
            }
            return Ok(json!({
                "kind": "question",
                "questions": normalized,
            }));
        }

        // 单问题模式：question + options（向后兼容）
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if question.is_empty() {
            return Err("缺少 question 或 questions 参数".to_string());
        }
        let header = args
            .get("header")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let multi = args
            .get("multiSelect")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let options = args
            .get("options")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let opts = self.normalize_options(&options)?;
        Ok(json!({
            "kind": "question",
            "questions": [{
                "id": format!("q_{}", uuid::Uuid::new_v4()),
                "header": header,
                "question": question,
                "multiSelect": multi,
                "options": opts,
            }],
        }))
    }

    /// 归一化单个问题对象（校验 options 数量 + 自动补 Other）
    fn normalize_question(&self, q: &Value) -> Result<Value, String> {
        let question = q
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if question.is_empty() {
            return Err("每个问题都需要 'question'".to_string());
        }
        let header = q
            .get("header")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let multi = q.get("multiSelect").and_then(|v| v.as_bool()).unwrap_or(false);
        let options = q
            .get("options")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let opts = self.normalize_options(&options)?;
        Ok(json!({
            "id": format!("q_{}", uuid::Uuid::new_v4()),
            "header": header,
            "question": question,
            "multiSelect": multi,
            "options": opts,
        }))
    }

    /// 校验选项数量（2~4），并自动追加 "Other" 选项
    fn normalize_options(&self, options: &[Value]) -> Result<Vec<Value>, String> {
        if options.len() < MIN_OPTIONS {
            return Err(format!("至少需要 {MIN_OPTIONS} 个选项"));
        }
        let mut opts: Vec<Value> = options
            .iter()
            .take(MAX_OPTIONS)
            .map(|o| {
                json!({
                    "label": o.get("label").and_then(|v| v.as_str()).unwrap_or(""),
                    "description": o.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                })
            })
            .collect();

        let has_other = opts.iter().any(|o| {
            o.get("label")
                .and_then(|v| v.as_str())
                .map(|s| s.eq_ignore_ascii_case("Other"))
                .unwrap_or(false)
        });
        if !has_other {
            opts.push(json!({
                "label": "Other",
                "description": "提供自定义输入"
            }));
        }
        Ok(opts)
    }
}

#[async_trait]
impl Capability for AskUserTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "ask_user".to_string(),
            description:
                "向用户提出结构化问题（含选项），用于在执行关键决策前获取用户输入。支持单问题(question)或批量(questions[], 1~4)，系统自动补充 'Other' 选项；完整交互需编排层阻塞等待用户选择。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "单问题模式：要问用户的问题" },
                    "questions": {
                        "type": "array",
                        "description": "批量模式：1~4 个问题（对齐 Trae AskUserQuestion）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": { "type": "string", "description": "要问用户的问题" },
                                "header": { "type": "string", "description": "问题短标签（如'部署方式'）" },
                                "multiSelect": { "type": "boolean", "description": "是否允许多选（默认 false）" },
                                "options": {
                                    "type": "array",
                                    "description": "选项数组（2~4 个，系统自动补 'Other'）",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": { "type": "string", "description": "选项显示文本" },
                                            "description": { "type": "string", "description": "选项说明" }
                                        },
                                        "required": ["label"]
                                    }
                                }
                            },
                            "required": ["question", "options"]
                        }
                    },
                    "header": { "type": "string", "description": "单问题模式：问题短标签" },
                    "multiSelect": { "type": "boolean", "description": "单问题模式：是否允许多选（默认 false）" },
                    "options": {
                        "type": "array",
                        "description": "单问题模式选项（2~4 个，系统自动补 'Other'）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "选项显示文本" },
                                "description": { "type": "string", "description": "选项说明" }
                            },
                            "required": ["label"]
                        }
                    }
                },
                "required": []
            }),
            category: Some(crate::symbio_core::CapabilityCategory::SystemOperation),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;

        let prompt = match self.build_prompt_payload(&args) {
            Ok(p) => p,
            Err(e) => {
                return Err(crate::symbio_core::PluginError::ValidationError(e));
            },
        };

        // 运行模式（auto/interactive，默认 interactive）：
        // - auto：无人值守，不产 user_prompt 节点，直接返回友好错误让 LLM 继续（不阻塞）。
        //   failure_kind=tool_unavailable 标记，前端可据此渲染（虽然不产节点，仅信息性）。
        // - interactive：会话流中渲染提问卡（user_prompt 节点），等待用户回答。
        let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();
        if mode == "auto" {
            return Ok(PluginPayload::new(&json!({
                "error": "当前为自动模式，不支持交互式提问（ask_user 不可用）。请基于已有上下文自行决策并继续，或提示用户切换到交互模式以获取交互提问能力。",
                "success": false,
                "failure_kind": "tool_unavailable",
            })));
        }

        // 通过 Session 通道广播一个 user_prompt 节点（parent_id = None，由 tool_executor 锚定到 tool_call_id）
        let (tx_side, rx_side) = PluginChannel::pair(16);
        let node = ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::UserPrompt),
            content: Some(MessageContent::Text("请回答问题以继续".to_string())),
            status: Some(MessageStatus::WaitingUserAction),
            meta: Some(json!({
                "prompt": prompt,
                // failure_kind=needs_interaction：前端据此渲染"填表提交"按钮（与错误盒统一）
                "failure_kind": "needs_interaction"
            })),
            ..Default::default()
        };
        let _ = tx_side
            .tx
            .send(PluginFrame::Data(
                serde_json::to_value(session_chat_response::StreamEvent::Update { message: node })
                    .unwrap_or_default(),
            ))
            .await;
        // 关闭发送侧，工具执行结束（编排层据此结束本轮并进入 AwaitingInput）
        drop(tx_side);
        Ok(PluginPayload::Session(rx_side))
    }
}
