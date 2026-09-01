use crate::plugins::agent::handlers::route as handlers_route;
use crate::plugins::agent::manager::validate_workspace_root;
use crate::plugins::agent::manager::AgentProfile;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::schemas::{
    model::model_chat,
    session::chat_message::{ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType},
    session::session_chat_response::StreamEvent,
};
use crate::symbio_core::CAPABILITY_AGENT_CHAT;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginChannel,
    PluginError, PluginFrame, PluginPayload,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

pub(crate) struct AgentChatTool {
    plugin: Arc<AgentPlugin>,
    cached_agents: Vec<AgentProfile>,
}

impl AgentChatTool {
    pub fn new(plugin: Arc<AgentPlugin>, agents: Vec<AgentProfile>) -> Self {
        Self {
            plugin,
            cached_agents: agents,
        }
    }

    fn format_agents_list(&self) -> String {
        if self.cached_agents.is_empty() {
            return "暂无可调用的智能体。\n使用 agent_create 创建新智能体。".to_string();
        }

        self.cached_agents
            .iter()
            .map(|agent| {
                let desc = if agent.description.is_empty() {
                    "(无描述)".to_string()
                } else {
                    agent.description.clone()
                };
                format!(
                    "   - ID: `{}`\n     Name: {}\n     Description: {}",
                    agent.id, agent.name, desc
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[async_trait]
impl Capability for AgentChatTool {
    fn meta(&self) -> CapabilityMeta {
        let agents_list = self.format_agents_list();

        CapabilityMeta {
            name: "agent_run".to_string(),
            description: format!(
                "通过对话，将任务委托给指定的智能体。\n\n\
                 ## 可用智能体列表\n\n\
                 {}\n\n\
                 ## 工作目录\n\n\
                 可选参数 `working_dir` 用于指定子智能体的工作目录：\n\
                 - 提供绝对路径（如 `/tmp/proj` 或 `C:\\repo\\myproj`），子智能体将在该目录独立工作\n\
                 - 不提供时，子智能体沿用当前智能体的工作目录\n\
                 - 禁止使用包含 `..` 的相对路径（系统会拒绝）",
                agents_list
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "要执行的智能体 ID"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "给智能体的清晰详细的提示或任务描述"
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "可选。子智能体的工作目录（绝对路径），省略则继承当前智能体的工作目录。\
                                支持 `~` 展开（如 `~/work/proj`），禁止包含 `..`。\
                                示例：`/tmp/proj`、`C:\\Users\\alice\\projects\\myproj`"
                    }
                },
                "required": ["agent_id", "prompt"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::Chat),
            examples: Some(vec![
                "agent_id='expert', prompt='分析这个项目的架构'".to_string(),
                "agent_id='expert', prompt='在 test 目录下创建 hello.py', working_dir='/tmp/test'".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        #[derive(serde::Deserialize, Clone)]
        struct RunRequest {
            agent_id: String,
            prompt: String,
            /// 子智能体的工作目录（绝对路径）。None = 继承父智能体工作目录。
            #[serde(default)]
            working_dir: Option<String>,
        }
        let req: RunRequest = ctx.payload()?;

        // ── 解析子智能体有效工作目录 ──
        // 优先级：用户显式参数 > 父 ctx[WORKDIR] > None（全局）
        // 显式参数必须经 `validate_workspace_root` 校验（拒绝 .. / 相对路径）
        let parent_workdir = ctx.get(crate::symbio_core::WORKDIR);
        let effective_workdir: Option<String> = match req.working_dir.as_deref() {
            Some(provided) => Some(validate_workspace_root(Some(provided)).ok_or_else(|| {
                PluginError::ValidationError(format!(
                    "Invalid working_dir '{}': must be an absolute path, \
                         `..` components are forbidden. \
                         Examples: `/tmp/proj`, `C:\\repo\\myproj`, `~/work`.",
                    provided
                ))
            })?),
            None => parent_workdir,
        };

        let tool_call_id = ctx.get(crate::symbio_core::TOOL_CALL_ID).ok_or_else(|| {
            PluginError::ValidationError("tool_call_id is required for run handler".to_string())
        })?;

        // 预先校验目标 agent 是否存在：不存在显式报错，
        // 绝不静默误用默认 agent（避免「agent_id 没生效 / 路由到默认 agent」类问题）。
        if self
            .plugin
            .get_mindscape(effective_workdir.as_deref(), &req.agent_id)
            .await
            .is_none()
        {
            return Err(PluginError::NotFound(format!(
                "目标智能体 '{}' 不存在，无法委托任务。请检查 agent_id 是否正确。",
                req.agent_id
            )));
        }

        let user_message = ChatMessage {
            id: tool_call_id,
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(req.prompt.clone())),
            ..Default::default()
        };

        let chat_req = model_chat::Request {
            system_prompt: None,
            single_message: Some(user_message),
            thinking: None,
            stream: Some(true),
            max_tool_rounds: Some(15),
            tool_context_window: Some(5),
            auto_compress: Some(true),
            provider_id: None,
            load_history: None,
            resume: None,
        };

        let chat_ctx = ctx.fork();
        chat_ctx.set(
            crate::symbio_core::SESSION_ID,
            uuid::Uuid::new_v4().to_string(),
        );
        chat_ctx.set(crate::symbio_core::AGENT_ID, req.agent_id);
        // 将子智能体的工作目录显式 set 到 fork ctx
        // （fork 虽会继承父 ctx 的 WORKDIR，但显式 set 覆盖能让语义更清晰：
        // 父 agent 在 workdir A、子 agent 在 workdir B 时，local 工具能拿到 B）
        if let Some(wd) = &effective_workdir {
            chat_ctx.set(crate::symbio_core::WORKDIR, wd.clone());
        }
        // 子会话继承父会话运行模式（auto 下子会话也不弹交互卡，避免嵌套阻塞）
        let parent_mode = ctx.get(crate::symbio_core::MODE);
        if let Some(m) = parent_mode {
            chat_ctx.set(crate::symbio_core::MODE, m);
        }
        // 子会话继承父会话执行风险等级（与 MODE 对称：父会话 risk_level=high 时子会话也允许高风险）
        let parent_risk_level = ctx.get(crate::symbio_core::RISK_LEVEL);
        if let Some(r) = parent_risk_level {
            chat_ctx.set(crate::symbio_core::RISK_LEVEL, r);
        }
        // 子会话继承父会话选定的 Model Provider ID（与 AGENT_ID 同级别对称）
        let parent_provider_id = ctx.get(crate::symbio_core::PROVIDER_ID);
        if let Some(p) = parent_provider_id {
            chat_ctx.set(crate::symbio_core::PROVIDER_ID, p);
        }
        chat_ctx.set_payload(chat_req)?;

        // 透传 effective_workdir 给 chat handler：让它的 mindscape / tools / workspace AGENTS.md
        // 全部按子智能体的工作目录解析（而非父智能体的）
        let resp = handlers_route(
            self.plugin.clone(),
            "chat",
            chat_ctx,
            effective_workdir.as_deref(),
        )
        .await?;

        let PluginPayload::Session(subagent_peer) = resp else {
            return Err(PluginError::InternalError(
                "Failed to start agent chat session for run".into(),
            ));
        };

        let (output_channel, my_side) = PluginChannel::pair(512);

        // 父会话 session_id：用于把子会话的 user_prompt 节点冒泡标记回父会话 UI
        let parent_session_id = ctx.get(crate::symbio_core::SESSION_ID);

        tokio::spawn(stream_relay(subagent_peer, my_side, parent_session_id));

        Ok(PluginPayload::Session(output_channel))
    }
}

// ── 流式转发 ──

/// I-049: 从子代理会话中接收流式事件，转发到输出通道，并累积最终文本结果。
///
/// 职责：
/// - 处理 `WaitingUserAction` 审批（自动批准）
/// - 累积 Assistant 文本消息（流式 → 完整）
/// - 转发所有 `StreamEvent` 到输出通道
/// - 在 idle / error 时结束，并发送最终结果
async fn stream_relay(
    mut subagent_peer: PluginChannel,
    my_side: PluginChannel,
    parent_session_id: Option<String>,
) {
    // 累积子 agent 各 Assistant 文本消息（按消息 id 分桶），并记录其出现顺序与是否已完成。
    // 最终 `final_result` 取「最后一条已完成的 Assistant 文本消息」内容，而不是简单地
    // 用「最后处理到的那条文本」覆盖——否则流式乱序到达的片段、或子 agent 的内部独白
    // （其自身对某工具调用的规划/思考，被当作 Text 发出）会被误当成最终答案塞进
    // agent_run 的工具结果里（表现为「结果里混入了子 agent 的工具调用信息 / 只有请求没有结果」）。
    let mut text_accumulator: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut completed_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut text_order: Vec<String> = Vec::new();
    let mut final_result = String::new();
    // 子 agent 以 PluginFrame::Error 结束时，结果应保留错误信息而非被最后文本覆盖
    let mut relay_error: Option<String> = None;

    while let Some(frame) = subagent_peer.rx.recv().await {
        match frame {
            PluginFrame::Data(val) => {
                if let Ok(event) = serde_json::from_value::<StreamEvent>(val) {
                    match event {
                        StreamEvent::Update { ref message } => {
                            // 子会话内的 user_prompt(WaitingUserAction) 节点（ask_user 提问 / 工具确认）
                            // 标记 subagent + 父会话 id，转发到父会话 UI 供用户填写（冒泡），
                            // 子会话本身在 chat_loop 层已结束本轮进入 AwaitingInput(user)；
                            // 用户在父会话视图回答后，答案作为 user 消息发回子会话，子会话继续。
                            // 详见 USER_INPUT_MECHANISM 设计文档 §4。
                            if message.msg_type == Some(MessageType::UserPrompt)
                                && message.status == Some(MessageStatus::WaitingUserAction)
                            {
                                let mut prompt_msg = message.clone();
                                if let Some(meta) = &mut prompt_msg.meta {
                                    if meta.is_object() {
                                        meta.as_object_mut().unwrap().insert(
                                            "subagent".to_string(),
                                            serde_json::Value::Bool(true),
                                        );
                                        if let Some(pid) = &parent_session_id {
                                            meta.as_object_mut().unwrap().insert(
                                                "parent_session_id".to_string(),
                                                serde_json::Value::String(pid.clone()),
                                            );
                                        }
                                    }
                                } else {
                                    let mut m = serde_json::Map::new();
                                    m.insert("subagent".to_string(), serde_json::Value::Bool(true));
                                    if let Some(pid) = &parent_session_id {
                                        m.insert(
                                            "parent_session_id".to_string(),
                                            serde_json::Value::String(pid.clone()),
                                        );
                                    }
                                    prompt_msg.meta = Some(serde_json::Value::Object(m));
                                }

                                let fwd = serde_json::to_value(StreamEvent::Update {
                                    message: prompt_msg,
                                })
                                .unwrap_or_default();
                                let _ = my_side.tx.send(PluginFrame::Data(fwd)).await;
                                // 继续转发其余事件，不暂停子会话链路
                                continue;
                            }

                            // 累积 Assistant 文本（排除 reasoning / 非文本）
                            if message.role == Some(crate::symbio_core::schemas::session::chat_message::MessageRole::Assistant)
                                && matches!(message.msg_type, Some(MessageType::Text) | None)
                            {
                                if let Some(content) = &message.content {
                                    let text = content.to_text();
                                    if !text.is_empty() {
                                        let id = message.id.clone();
                                        if !text_accumulator.contains_key(&id) {
                                            text_order.push(id.clone());
                                        }
                                        let buf = text_accumulator.entry(id.clone()).or_default();
                                        if message.status == Some(MessageStatus::Streaming) {
                                            buf.push_str(&text);
                                        } else {
                                            *buf = text;
                                            completed_ids.insert(id);
                                        }
                                    }
                                }
                            }

                            let fwd = serde_json::to_value(StreamEvent::Update {
                                message: message.clone(),
                            })
                            .unwrap_or_default();
                            let _ = my_side.tx.send(PluginFrame::Data(fwd)).await;
                        }

                        StreamEvent::Error { ref error } => {
                            let fwd = serde_json::to_value(StreamEvent::Error {
                                error: error.clone(),
                            })
                            .unwrap_or_default();
                            let _ = my_side.tx.send(PluginFrame::Data(fwd)).await;
                            break;
                        }

                        StreamEvent::Status { ref status } if status.as_str() == "idle" => {
                            break;
                        }

                        _ => {}
                    }
                }
            }
            PluginFrame::Error(e, _) => {
                relay_error = Some(format!("Error: {e}"));
                break;
            }
        }
    }

    // 子 agent 以错误结束：直接返回错误信息（不被最后文本覆盖）
    if let Some(err) = relay_error {
        final_result = err;
    } else {
        // （例如子 agent 仅产出 reasoning 而未产出文本），则退回最后一条累积到的文本，
        // 保证 agent_run 的结果不为空（避免出现「只有请求没有结果」）。
        if let Some(id) = text_order
            .iter()
            .rev()
            .find(|id| completed_ids.contains(*id))
            .or(text_order.last())
        {
            final_result = text_accumulator.get(id).cloned().unwrap_or_default();
        }
    }

    let _ = my_side
        .tx
        .send(PluginFrame::Data(json!({ "content": final_result })))
        .await;
    drop(my_side);
}

// ═══════════════════════════════════════════════════════════════════════════
// 工厂 + 自注册
// ---------------------------------------------------------------------
// 工厂签名遵循 `submit_object_creator!` 的统一协议：
// `fn(Arc<dyn InvokeRequest>) -> Arc<dyn Capability>`
// 运行期依赖（`AgentPlugin` + 智能体列表）通过 `AGENT_CAPABILITY_CONTEXT` 键
// 由 `plugin.rs::traverse` 注入；本工厂只负责取出并构造。
// ═══════════════════════════════════════════════════════════════════════════

pub fn build_chat(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Capability> {
    let cap_ctx = crate::plugins::agent::capabilities::get_capability_context(ctx.as_ref());
    Arc::new(AgentChatTool::new(
        cap_ctx.plugin.clone(),
        cap_ctx.agents.clone(),
    )) as Arc<dyn Capability>
}

crate::submit_object_creator!(CAPABILITY_AGENT_CHAT, build_chat, dyn Capability);
