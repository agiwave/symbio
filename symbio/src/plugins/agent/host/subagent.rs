//! agent_run —— 委托型工具：把任务交给另一个已安装的 bundle（子智能体）执行。
//!
//! ## 架构定位：普通工具，零特殊化
//!
//! agent_run 与 file_read / shell 等工具完全同构：
//! - 经 `traverse(available_tools)` 注册（Capability 机制，见 `super::plugin`）；
//! - 返回 `PluginPayload::Session` 流式通道，前端按普通流式工具渲染；
//! - 子会话内工具需要用户审批时，产出 `user_prompt(WaitingUserAction)`——与
//!   confirm 类工具走**同一套**审批机制（`session/tool_executor` 捕获 →
//!   父会话等待 → `session/resume` 重执行本工具）；
//! - 无注册表、无唤醒队列、无前端专属概念。"子会话"只是本工具的内部实现：
//!   一个由 session 插件托管、按 `metadata.parent_session_id` 归档到父会话
//!   `sessions/` 子目录的普通会话（归属机制见 `session/store/file`）。
//!
//! ## 执行流程（三种调用形态）
//!
//! 首次调用（args: `agent_id` / `prompt` / `working_dir?` / `session_id?`）：
//! 1. 校验目标 bundle 存在、`working_dir` 合法；
//! 2. `session/update` 落子会话元数据（`parent_session_id` → 存储归档父会话
//!    子目录、不进用户会话列表）；
//! 3. 路由 `session/chat/send`（统一编排入口：collect → traverse 重新装配工具，
//!    子会话与顶层会话走同一条链路）；
//! 4. 订阅 EventBus，把子会话事件流转播到工具输出通道（父会话 UI 锚定到
//!    本工具调用之下）；
//! 5. 子会话完成（idle 无待审批）→ 工具结果 = 最终文本（末尾附
//!    `[subagent_session_id]`，供 LLM 后续续会话）；等待审批 →
//!    user_prompt 冒泡（覆写 `meta.prompt` 为本工具续跑参数）→ 工具以待审批结束。
//!
//! 续会话（args 另带 `session_id`，LLM 携带上次结果中的子会话 id）：
//! 存在性校验后按 3-5 把 prompt 作为该子会话的下一轮消息——委托对话上下文
//! 自然延续，LLM 无需转述历史。
//!
//! 恢复调用（用户在父会话审批后，`session/resume` 按普通工具恢复机制重执行
//! 本工具，args 追加 `session_id` / `target_id` / `approved`）：
//! 路由 `session/chat/send` 的 resume 分支到子会话（在子会话内恢复被审批的
//! 工具），随后同 4-5。父子关系始终由子会话存储元数据承载，无进程内状态。

use super::store::BundleStore;
use crate::symbio_core::event_bus::{register_subscriber, unregister_subscriber};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType, ResumeAction,
    ResumeRequest,
};
use crate::symbio_core::schemas::session::session_chat;
use crate::symbio_core::schemas::session::session_chat_response::StreamEvent;
use crate::symbio_core::schemas::session::session_get_messages;
use crate::symbio_core::schemas::session::session_update;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginChannel, PluginError,
    PluginFrame, PluginPayload, MODE, PATH, PROVIDER_ID, RISK_LEVEL, SESSION_ID, TOOL_CALL_ID,
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

// ── 本工具的 meta 键约定（续跑参数承载于审批节点的 meta.prompt.args）──
/// 审批节点 meta.prompt：resume 重执行工具时的参数来源（session/resume 提取）
const META_PROMPT: &str = "prompt";
/// 子会话元数据键：父会话 id（session 存储归档依据）
const KEY_PARENT_SESSION_ID: &str = "parent_session_id";

/// agent_run 能力：把任务委托给指定 bundle（子智能体）。
pub struct AgentRunCapability {
    /// 子会话默认 workdir（构造时解析：父 ctx > 无）
    workdir: Option<String>,
    /// 工具描述中的可用 bundle 清单（供 LLM 选择 agent_id）
    bundles_brief: String,
    /// 插件间路由入口（composite 容器弱引用，构造时由插件实例捕获）
    router: Option<std::sync::Weak<dyn Plugin>>,
}

impl AgentRunCapability {
    pub fn new(
        workdir: Option<String>,
        bundles_brief: String,
        router: Option<std::sync::Weak<dyn Plugin>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            workdir,
            bundles_brief,
            router,
        })
    }
}

#[async_trait::async_trait]
impl crate::symbio_core::Capability for AgentRunCapability {
    fn meta(&self) -> crate::symbio_core::CapabilityMeta {
        crate::symbio_core::CapabilityMeta {
            name: "agent_run".to_string(),
            context_retention: None,
            description: format!(
                "将任务委托给一个独立的子智能体会话：在其独立会话中执行任务，\
                 完成后返回其最终答复；子智能体拥有自己的提示词与工具集。\n\n\
                 ## 智能体选择\n\n\
                 `agent_id` 可选：省略时默认使用当前会话正在使用的智能体；\
                 若当前会话也未绑定智能体，则子会话以无智能体的纯对话模式运行。\n\n\
                 ## 可用智能体列表\n\n{}\n\n\
                 ## 工作目录\n\n\
                 可选参数 `working_dir` 指定子智能体的工作目录：绝对路径、支持 `~` 展开、\
                 禁止包含 `..`；省略时沿用当前会话的工作目录。",
                self.bundles_brief
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "可选。要执行的智能体 ID（bundle id，见可用智能体列表）；\
                                省略则默认使用当前会话正在使用的智能体"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "给智能体的清晰详细的提示或任务描述"
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "可选。子智能体的工作目录（绝对路径），省略则继承当前会话的工作目录。\
                                支持 `~` 展开（如 `~/work/proj`），禁止包含 `..`。\
                                示例：`/tmp/proj`、`C:\\Users\\alice\\projects\\myproj`"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "可选。继续之前的委托对话：传入上次工具结果末尾的 \
                                [subagent_session_id]，本次 prompt 将作为该对话的下一轮，\
                                保留全部上下文；省略则新开一个子会话。"
                    }
                },
                "required": ["prompt"]
            }),
            keywords: Vec::new(),
            category: Some(crate::symbio_core::CapabilityCategory::Chat),
            examples: Some(vec![
                "agent_id='<bundle-id>', prompt='分析这个项目的架构'".to_string(),
            ]),
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        #[derive(serde::Deserialize, Clone)]
        struct RunRequest {
            /// 可选：目标智能体 id（bundle id）。省略 → 默认用当前会话的智能体
            ///（ctx[AGENT_ID]）；两者皆空 → 子会话以无智能体的纯对话模式运行。
            #[serde(default)]
            agent_id: Option<String>,
            /// 首次调用必填；恢复调用（审批后续跑）可缺省
            #[serde(default)]
            prompt: String,
            /// 子智能体的工作目录（绝对路径）。None = 继承父会话工作目录。
            #[serde(default)]
            working_dir: Option<String>,
            // ── 以下为续会话 / 恢复调用字段 ──
            /// 续会话（LLM 传：继续之前的委托对话）或审批恢复（resume 机制
            /// 透传）时的子会话 id；省略则新开子会话
            #[serde(default)]
            session_id: Option<String>,
            /// 子会话内待恢复节点 id（仅审批恢复时由 resume 机制注入）
            #[serde(default)]
            target_id: Option<String>,
            /// 用户审批结果（session/resume 的 Approve 分支注入）
            #[serde(default)]
            approved: Option<bool>,
        }
        let req: RunRequest = ctx.payload()?;

        if ctx.get(TOOL_CALL_ID).is_none() {
            return Err(PluginError::ValidationError(
                "agent_run 需要 tool_call_id 上下文".to_string(),
            ));
        }
        let parent_session_id = ctx.get(SESSION_ID).unwrap_or_default();
        if parent_session_id.is_empty() {
            return Err(PluginError::ValidationError(
                "agent_run 需要会话上下文（SESSION_ID）".to_string(),
            ));
        }
        let parent = self
            .router
            .as_ref()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| {
                PluginError::InternalError(
                    "agent_run 无法获取路由入口（插件构造时未捕获 PARENT 引用）".to_string(),
                )
            })?;

        // ── 解析目标智能体：显式参数 > 当前会话绑定的智能体 > None（纯对话子会话）──
        let current_agent = ctx
            .get(crate::symbio_core::AGENT_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let agent_id: Option<String> = req
            .agent_id
            .clone()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or(current_agent);

        // ── 解析有效工作目录（显式参数 > 父会话继承），并校验目标 bundle 存在 ──
        // 绝不静默降级：bundle 缺失 / working_dir 非法直接报错回传 LLM。
        let effective_workdir: Option<String> = match req.working_dir.as_deref() {
            Some(provided) => Some(validate_working_dir(provided)?),
            None => self.workdir.clone(),
        };
        if let Some(aid) = &agent_id {
            let store = BundleStore::new(effective_workdir.as_deref());
            if store.get(aid).is_none() {
                return Err(PluginError::NotFound(format!(
                    "目标智能体 '{aid}' 不存在，无法委托任务。请检查 agent_id 是否正确。"
                )));
            }
        }

        // ── 子会话 id + 续跑请求 + 首条消息（三者按调用形态确定）──
        // 三种调用形态：审批恢复（session_id + target_id）/ 续会话（仅
        // session_id，LLM 携带上次结果的子会话 id 继续对话）/ 新建（都省略）。
        let user_message = || ChatMessage {
            id: uuid::Uuid::new_v4().as_simple().to_string(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(req.prompt.clone())),
            ..Default::default()
        };
        let (child_session_id, resume_req, first_message): (
            String,
            Option<ResumeRequest>,
            Option<ChatMessage>,
        ) = match (req.session_id.as_deref(), req.target_id.as_deref()) {
            (Some(sid), Some(tid)) if !sid.is_empty() && !tid.is_empty() => {
                // 恢复调用：在子会话内恢复被审批的工具（普通 resume 语义透传）。
                let action = if req.approved == Some(true) {
                    ResumeAction::Approve
                } else {
                    ResumeAction::Retry
                };
                (
                    sid.to_string(),
                    Some(ResumeRequest {
                        target_id: tid.to_string(),
                        action,
                        args: None,
                        reason: None,
                        answer: None,
                    }),
                    None,
                )
            }
            (Some(sid), None) if !sid.is_empty() => {
                // 续会话：prompt 作为该子会话的下一轮用户消息，上下文自然延续。
                validate_subsession_exists(parent.clone(), &ctx, sid).await?;
                (sid.to_string(), None, Some(user_message()))
            }
            _ => {
                // 首次调用：创建子会话并落元数据。
                // `parent_session_id` 由 session 存储解释为归档位置
                // （父会话目录的 sessions/ 子目录）与列表过滤依据；
                // `agent_id` / `workdir` 供 chat 编排回退链与详情展示。
                let sid = uuid::Uuid::new_v4().to_string();
                let upd_ctx = ctx.fork();
                upd_ctx.set(PATH, "session/update".to_string());
                let _ = upd_ctx.set_payload(session_update::Request {
                    session_id: sid.clone(),
                    metadata: json!({
                        KEY_PARENT_SESSION_ID: parent_session_id,
                        "agent_id": agent_id,
                        "workdir": effective_workdir,
                        "title": format!("↳ {}", agent_id.as_deref().unwrap_or("子智能体")),
                    }),
                    title: None,
                });
                if let Err(e) = parent.clone().route(upd_ctx).await {
                    crate::plugin_warn!("agent", "登记子会话元数据失败（不影响委托执行）: {}", e);
                }
                (sid, None, Some(user_message()))
            }
        };

        // ── 订阅子会话事件流（先于 chat/send，避免丢首帧）──
        // EventBus 会推送所有会话的帧，中继按 session_id 过滤出本工具的子会话。
        let (bus_tx, bus_rx) = tokio::sync::mpsc::channel::<PluginFrame>(1024);
        let conn_id = format!("agent-run-{child_session_id}");
        register_subscriber(conn_id.clone(), bus_tx);

        // ── 路由 session/chat/send（统一编排入口）──
        // mode / risk_level / provider_id 随请求显式继承（resolve_session_params
        // 的回退链是 req > metadata > 默认，ctx 键会被覆盖，必须走 req 字段）。
        let send_ctx = ctx.fork();
        send_ctx.set(PATH, "session/chat/send".to_string());
        send_ctx.set(SESSION_ID, child_session_id.clone());
        let _ = send_ctx.set_payload(session_chat::Request {
            session_id: Some(child_session_id.clone()),
            agent_id: agent_id.clone(),
            message: first_message,
            provider_id: ctx.get(PROVIDER_ID),
            include_history: None,
            mode: ctx.get(MODE),
            risk_level: ctx.get(RISK_LEVEL),
            resume: resume_req,
        });

        if let Err(e) = parent.route(send_ctx).await {
            // 委托失败：清理订阅，错误直接作为工具结果回传
            unregister_subscriber(&conn_id);
            return Err(e);
        }

        // ── 工具输出通道 + 事件转播 ──
        let (output_chan, my_side) = PluginChannel::pair(512);
        tokio::spawn(stream_relay_bridge(
            bus_rx,
            my_side,
            conn_id,
            child_session_id,
            agent_id,
        ));
        Ok(PluginPayload::Session(output_chan))
    }
}

/// 校验 working_dir：绝对路径、拒绝 `..` 段、支持 `~` 展开。
fn validate_working_dir(provided: &str) -> Result<String, PluginError> {
    let trimmed = provided.trim();
    if trimmed.is_empty() {
        return Err(PluginError::ValidationError(
            "working_dir 不能为空".to_string(),
        ));
    }
    if trimmed.split(['/', '\\']).any(|seg| seg == "..") {
        return Err(PluginError::ValidationError(format!(
            "Invalid working_dir '{provided}': `..` components are forbidden."
        )));
    }
    let expanded = crate::symbio_core::expand_tilde_path(std::path::Path::new(trimmed));
    if !expanded.is_absolute() {
        return Err(PluginError::ValidationError(format!(
            "Invalid working_dir '{provided}': must be an absolute path. \
             Examples: `/tmp/proj`, `C:\\repo\\myproj`, `~/work`."
        )));
    }
    Ok(expanded.to_string_lossy().to_string())
}

/// 续会话存在性轻校验：委托会话首条即用户消息，消息列表为空即"不存在"。
///
/// `PersistentChatSession` 是惰性句柄（`session/open` 永远成功），无法直接探测
/// 文件存在性；该校验防止 LLM 拼错 session_id 时 `get_or_create_session`
/// 静默创建出一个顶层孤儿会话。
async fn validate_subsession_exists(
    parent: Arc<dyn Plugin>,
    ctx: &Arc<dyn InvokeRequest>,
    session_id: &str,
) -> Result<(), PluginError> {
    let gm_ctx = ctx.fork();
    gm_ctx.set(PATH, "session/get_messages".to_string());
    let _ = gm_ctx.set_payload(session_get_messages::Request {
        session_id: session_id.to_string(),
    });
    let payload = parent.route(gm_ctx).await?;
    let exists = match &payload {
        PluginPayload::Data(data) => data
            .downcast_ref::<session_get_messages::Response>()
            .map(|r| !r.messages.is_empty())
            .unwrap_or(false),
        _ => false,
    };
    if !exists {
        return Err(PluginError::NotFound(format!(
            "子会话 '{session_id}' 不存在或无消息记录，无法继续该对话。\
             请省略 session_id 参数以新开一个委托会话。"
        )));
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 事件转播
// ---------------------------------------------------------------------
// 桥接：session 编排（one-off，事件经 EventBus 以子会话 id 推送）↔
// 本工具的输出通道（execute_tool_async 的 Session 载荷消费端）。
// 与普通流式工具的差别仅在于数据源是 EventBus 而非内部通道——对父会话
// 而言这就是一次普通的流式工具调用。
// ═══════════════════════════════════════════════════════════════════════════

/// 事件转播任务。
///
/// - 消费 EventBus 帧，按 `session_id == child_session_id` 过滤出子会话事件；
/// - 转发 `StreamEvent::Update` 到工具输出通道（父会话 UI 锚定到工具调用之下）；
/// - 审批透传：子会话的 `user_prompt(WaitingUserAction)` 覆写 `meta.prompt`
///   为本工具续跑参数后转发——tool_executor 将其捕获为本轮"待用户响应"结果，
///   父会话进入普通工具审批等待；
/// - 累积 Assistant 文本，子会话 idle / error 时结束并发送 `{"content": final}`。
///
/// 收尾后通道关闭：等待审批时 execute_tool_async 以捕获的 user_prompt 结束本轮
/// （普通机制）；正常完成时以最终文本结束（父会话 LLM 拿到结果自然继续）。
async fn stream_relay_bridge(
    mut bus_rx: tokio::sync::mpsc::Receiver<PluginFrame>,
    out_chan: PluginChannel,
    conn_id: String,
    child_session_id: String,
    agent_id: Option<String>,
) {
    // 累积 Assistant 文本（按消息 id 分桶）：最终结果取「最后一条已完成的
    // Assistant 文本」而非「最后处理到的文本」——流式乱序片段或子 agent 的
    // 内部独白不能被误当成最终答案（老版 I-049 修复语义）。
    let mut text_accumulator: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut completed_ids: HashSet<String> = HashSet::new();
    let mut text_order: Vec<String> = Vec::new();
    let mut final_result = String::new();
    let mut relay_error: Option<String> = None;

    while let Some(frame) = bus_rx.recv().await {
        let PluginFrame::Data(envelope) = frame else {
            continue;
        };
        // envelope = {type:"bus_event", data:{kind, session_id, data}}
        let Some(bus) = envelope.get("data") else {
            continue;
        };
        if bus.get("kind").and_then(|v| v.as_str()) != Some("session") {
            continue;
        }
        if bus.get("session_id").and_then(|v| v.as_str()) != Some(child_session_id.as_str()) {
            continue;
        }
        let Some(payload) = bus.get("data").cloned() else {
            continue;
        };
        let Ok(event) = serde_json::from_value::<StreamEvent>(payload) else {
            continue;
        };

        match event {
            StreamEvent::Update { ref message } => {
                // ── 审批透传：子会话内工具需要用户审批 ──
                // 覆写 meta.prompt 为本工具的续跑参数（session/resume 重执行
                // agent_run 时据此续跑子会话）；其余 meta 原样保留
                //（failure_kind 等驱动前端审批 UI）。
                if message.msg_type == Some(MessageType::UserPrompt)
                    && message.status == Some(MessageStatus::WaitingUserAction)
                {
                    let mut bubble = message.clone();
                    let obj = bubble
                        .meta
                        .get_or_insert_with(|| json!({}))
                        .as_object_mut()
                        .expect("meta 保证为 object");
                    obj.insert(
                        META_PROMPT.to_string(),
                        json!({
                            "tool_name": "agent_run",
                            "args": {
                                "agent_id": agent_id,
                                "session_id": child_session_id,
                                "target_id": message.id,
                            }
                        }),
                    );
                    let fwd = serde_json::to_value(StreamEvent::Update { message: bubble })
                        .unwrap_or_default();
                    if send_frame(&out_chan, fwd).is_err() {
                        break;
                    }
                    // 继续转发其余事件（子会话本轮很快以 idle 结束）
                    continue;
                }

                // ── 累积 Assistant 文本（排除 reasoning / 非文本）──
                if message.role == Some(MessageRole::Assistant)
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

                let fwd =
                    serde_json::to_value(StreamEvent::Update { message: message.clone() })
                        .unwrap_or_default();
                if send_frame(&out_chan, fwd).is_err() {
                    // 工具通道已关闭（父会话被中止/重试）：停止转播
                    break;
                }
            }
            StreamEvent::Error { ref error } => {
                let fwd = serde_json::to_value(StreamEvent::Error {
                    error: error.clone(),
                })
                .unwrap_or_default();
                let _ = send_frame(&out_chan, fwd);
                relay_error = Some(error.clone());
                break;
            }
            StreamEvent::Status { ref status } if status.as_str() == "idle" => {
                break;
            }
            _ => {}
        }
    }

    // ── 收尾：确定工具结果文本 ──
    if let Some(err) = relay_error {
        final_result = err;
    } else if let Some(id) = text_order
        .iter()
        .rev()
        .find(|id| completed_ids.contains(*id))
        .or(text_order.last())
    {
        final_result = text_accumulator.get(id).cloned().unwrap_or_default();
    }
    if final_result.is_empty() {
        final_result = "（子智能体已结束，未产生文本输出）".to_string();
    }
    // 附加子会话 id：LLM 续会话的凭据（下一次调用传 session_id 即继续此对话）。
    // 审批中断时本 content 会被 tool_executor 以捕获的 user_prompt 替代，无副作用。
    final_result.push_str(&format!("\n\n[subagent_session_id: {child_session_id}]"));

    unregister_subscriber(&conn_id);

    let _ = out_chan
        .tx
        .send(PluginFrame::Data(json!({ "content": final_result })))
        .await;
    drop(out_chan);
}

/// 发送帧到工具输出通道；对端关闭返回 Err。
fn send_frame(chan: &PluginChannel, data: serde_json::Value) -> Result<(), ()> {
    chan.tx.try_send(PluginFrame::Data(data)).map_err(|_| ())
}

/// 从 BundleStore 摘要生成工具描述中的"可用智能体列表"。
pub(crate) fn format_bundles_brief(bundles: &[super::store::BundleRecord]) -> String {
    if bundles.is_empty() {
        return "（暂无可用智能体）".to_string();
    }
    bundles
        .iter()
        .map(|r| {
            let desc = if r.manifest.description.is_empty() {
                "(无描述)".to_string()
            } else {
                r.manifest.description.clone()
            };
            format!(
                "- ID: `{}`\n  Name: {}\n  Description: {}",
                r.manifest.id, r.manifest.name, desc
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
