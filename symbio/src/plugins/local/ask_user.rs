//! 询问用户工具 - 实现 Capability（对应 Trae 的 AskUserQuestion）
//!
//! 支持单问题（question）或批量问题（questions[]，1~4 个，对齐 Trae）。
//!
//! **本工具不构造节点**：它只返回 `failure_kind = needs_interaction` 加 `prompt` 载荷，
//! 由编排层（`session/tool_executor.rs`）构造 `user_prompt` 节点
//! （status = WaitingUserAction，id = 工具结果占位节点 id）并将会话置于
//! `AwaitingInput(user)`；用户答案以一条普通 `user` 消息（`meta.responds_to` 指向该节点）
//! 回填后，新一轮会重跑本工具并拿到答案。options 自动补充 "Other" 选项。
//!
//! 与 `local` 的 confirm 流程是**同一机制的两种问法**
//! （见 `plugin.rs::confirm_prompt_payload`）：confirm 问「是否允许执行某工具」，
//! 本工具问「请回答问题」；两者返回的载荷形状一致（`prompt` + `failure_kind`），
//! 前端的提问卡与回答回填链路共用。自动模式（`mode == "auto"`）下不产节点，
//! 返回 `tool_unavailable` 让 LLM 自行继续，避免无人值守时阻塞。

use crate::symbio_core::{
    Capability, CapabilityMeta, ExecEnv, InvokeRequest, InvokeRequestExt, PluginError,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_QUESTIONS: usize = 4;
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 4;

/// 询问用户工具。本身不接触文件系统，故不持有 `SecurityPolicy`。
#[derive(Clone, Default)]
pub struct AskUserTool;

impl AskUserTool {
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
        let multi = q
            .get("multiSelect")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
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

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn InvokeRequest>,
    ) -> Result<Value, PluginError> {
        let prompt = match self.build_prompt_payload(&args) {
            Ok(p) => p,
            Err(e) => {
                return Err(crate::symbio_core::PluginError::ValidationError(e));
            }
        };

        // 运行模式（auto/interactive，默认 interactive）：
        // - auto：无人值守，不产 user_prompt 节点，直接返回友好错误让 LLM 继续（不阻塞）。
        //   failure_kind=tool_unavailable 标记，前端可据此渲染（虽然不产节点，仅信息性）。
        // - interactive：会话流中渲染提问卡（user_prompt 节点），等待用户回答。
        let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();
        if mode == "auto" {
            return Ok(json!({
                "error": "当前为自动模式，不支持交互式提问（ask_user 不可用）。请基于已有上下文自行决策并继续，或提示用户切换到交互模式以获取交互提问能力。",
                "success": false,
                "failure_kind": crate::symbio_core::failure_kind::TOOL_UNAVAILABLE,
            }));
        }

        // 只回答「要问用户什么」——`prompt` 载荷 + `failure_kind`。
        // **user_prompt 节点由编排层构造**（`session/tool_executor.rs`）：它拥有
        // `result_msg_id`（节点身份）与父 ToolCall 终态，是工具结果节点的唯一写入者。
        //
        // 历史上这里建一条 Session 通道、发一个 Upsert 帧、drop tx 再返回 rx，
        // 由消费方解回来、把 parent_id 锚到 tool_call_id、改 id 后重新播一遍——
        // 同一个逻辑节点因此有两个 id，审批 UI 重复且 resume 只删得掉一个。
        Ok(json!({
            "content": "请回答问题以继续",
            "success": false,
            // 编排层凭此标记把本轮收口为「等待用户动作」并构造 user_prompt 节点。
            "failure_kind": crate::symbio_core::failure_kind::NEEDS_INTERACTION,
            "prompt": prompt,
        }))
    }
}

#[cfg(test)]
#[path = "ask_user.test.rs"]
mod tests;
