//! `model.assist` 端口成员：**一次性问答**（D-336）。
//!
//! 与 `model` 端口分开是有判据的：`model` 是对话轮（带工具表、带会话历史、流式帧进
//! 展示面）；`model.assist` 只把一份问答请求发出去、取回正文——不构对话、不带工具、
//! 不留历史。执行体归本插件（端口提供方），提问方的指令随行载荷进来，本插件不认识
//! 「压缩」这类世界语义。
//!
//! ## 载荷
//!
//! [`AssistRequest`]：`system`（提问方指令）+ `user`（正文）。`user` 为空时从事实读面
//! 拼本轮转写（用户消息 + 助手终答）——载荷只带身份，正文不进载荷。
//!
//! ## 应答
//!
//! 返回一条载荷 [`AssistReply`] + `root_id` / `final_id`：环把它落成 `response`
//! 事实并补 `port`，读方按 `port == model.assist` 取回。`response` 不进
//! `turn_input` 的 messages（那里只收 `port == model`），所以问答不会污染对话历史。

use std::sync::Arc;

use serde_json::json;
use symbio_plugin_api::chat_node::{
    AssistReply, AssistRequest, ChatNode, MessageContent, NodeRole,
};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::port::{
    port_rows, PortErrorClass, PortExecContext, PortExecutor, PortFact, PortInput, PortOutcome,
    PortProvider,
};
use symbio_plugin_api::{
    llm_short_id, CapabilityRow, ExecEnv, ExecEventSink, ModelProvider, PluginError,
};

/// 端口名（数据；提供者自己定义，契约层不认识它）。
pub(crate) const ASSIST_PORT: &str = "model.assist";

/// 提供者 id（行名后缀；与 `model` 端口行的提供者区分）。
const ASSIST_PROVIDER: &str = "model-assist";

pub(crate) struct AssistPort {
    active: Arc<dyn ModelProvider>,
}

impl AssistPort {
    pub(crate) fn new(active: Arc<dyn ModelProvider>) -> Self {
        Self { active }
    }

    pub(crate) fn rows(self: Arc<Self>) -> Vec<CapabilityRow> {
        port_rows(&(self as Arc<dyn PortProvider>))
    }
}

impl PortProvider for AssistPort {
    fn ports(&self) -> &'static [&'static str] {
        &[ASSIST_PORT]
    }

    fn provider_id(&self) -> &str {
        ASSIST_PROVIDER
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(AssistExecutor {
            active: self.active.clone(),
        }))
    }
}

struct AssistExecutor {
    active: Arc<dyn ModelProvider>,
}

#[async_trait::async_trait]
impl PortExecutor for AssistExecutor {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        // 共享载荷是唯一的输入形状；无法反序列化 ⇒ 这一笔响亮失败。
        let request: AssistRequest = match serde_json::from_value(input.payload.clone()) {
            Ok(request) => request,
            Err(error) => {
                return PortOutcome::failed(
                    PortErrorClass::NonRetryable,
                    format!("问答载荷不可读：{error}"),
                )
            }
        };
        let root_id = input
            .payload
            .get("root_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let final_id = input
            .payload
            .get("final_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();

        let user = if request.user.trim().is_empty() {
            let facts = exec
                .reader
                .as_ref()
                .map(|reader| reader.facts())
                .unwrap_or_default();
            transcript(&facts)
        } else {
            request.user.clone()
        };
        if user.trim().is_empty() {
            return PortOutcome::ok();
        }

        // 一条消息、无工具表、静默出口：问答不参与对话轮，也不往展示面写帧。
        let wire = vec![ChatNode {
            id: llm_short_id(),
            role: Some(NodeRole::User),
            content: Some(MessageContent::Text(user)),
            ..Default::default()
        }];
        let env = ExecEnv::new(ExecEventSink::silent(), exec.abort.clone());
        match self
            .active
            .round_trip(&request.system, &wire, &[], &root_id, &env)
            .await
        {
            Ok(output) => {
                let reply = AssistReply {
                    text: output.text.trim().to_string(),
                };
                PortOutcome {
                    facts: vec![json!({
                        "root_id": root_id,
                        "final_id": final_id,
                        "text": reply.text,
                    })],
                    ..Default::default()
                }
            }
            Err(PluginError::RateLimited(why)) => {
                PortOutcome::failed(PortErrorClass::Retryable, why)
            }
            Err(error) => PortOutcome::failed(PortErrorClass::NonRetryable, error.to_string()),
        }
    }
}

/// 本轮转写：用户发言 + 助手终答，按事实顺序拼成一问一答的正文。
fn transcript(facts: &[PortFact]) -> String {
    let mut out = String::new();
    for fact in facts {
        let (role, text) = match fact.kind.as_str() {
            EVENT_USER_MESSAGE => ("用户", fact.payload.get("text")),
            EVENT_CHAT_ASSISTANT_FINAL => (
                "助手",
                fact.payload
                    .get("text")
                    .or_else(|| fact.payload.get("answer")),
            ),
            _ => continue,
        };
        let Some(text) = text.and_then(serde_json::Value::as_str) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(role);
        out.push('：');
        out.push_str(text.trim());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transcript_reads_user_and_final_facts() {
        let facts = vec![
            PortFact {
                id: "u-1".into(),
                port: String::new(),
                kind: EVENT_USER_MESSAGE.into(),
                payload: json!({ "text": "问题一" }),
            },
            PortFact {
                id: "chat.assistant.final:u-1".into(),
                port: "model".into(),
                kind: EVENT_CHAT_ASSISTANT_FINAL.into(),
                payload: json!({ "text": "答复一" }),
            },
        ];
        assert_eq!(transcript(&facts), "用户：问题一\n助手：答复一\n");
    }
}
