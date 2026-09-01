//! Local Tools 插件实现

use super::policy::{RiskLevel, SecurityPolicy};
use super::{
    ask_user::AskUserTool, codebase_search::CodebaseSearchTool, content_search::ContentSearchTool,
    dir_list::DirListTool, file_delete::FileDeleteTool, file_edit::FileEditTool,
    file_read::FileReadTool, file_search::FileSearchTool, file_write::FileWriteTool,
    shell::ShellTool, todo_write::TodoWriteTool,
};
pub use crate::symbio_core::schemas::agent::local_config::LocalConfig;
use crate::symbio_core::schemas::common::SimpleResponse;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::schemas::session::session_chat_response;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginChannel, PluginError, PluginFrame, PluginMeta, PluginPayload, CONFIG_GET, CONFIG_SET,
    PLUGIN_LOCAL,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// 从 ctx[RISK_LEVEL] 读取 per-session 风险等级阈值。
///
/// 与 agent_id/provider_id/mode 同级别：随 chat_send 传输，由 orchestrator 写入 ctx。
/// ctx 无值时默认 `Medium`（新会话尚未设置时的安全默认值）。
fn risk_level_from_ctx(ctx: &Arc<dyn InvokeRequest>) -> RiskLevel {
    ctx.get(crate::symbio_core::RISK_LEVEL)
        .map(|s| match s.as_str() {
            "low" => RiskLevel::Low,
            "high" => RiskLevel::High,
            _ => RiskLevel::Medium,
        })
        .unwrap_or(RiskLevel::Medium)
}

/// 构造一个 confirm 类型的 `user_prompt` 节点，并通过 Session 通道广播。
/// 调用方据此结束本轮，会话进入 AwaitingInput(user)；用户批准后新一轮重跑本工具。
async fn emit_confirm_prompt(
    tool_name: &str,
    tool_description: &str,
    args: &Value,
    risk_level: &str,
    mode: &str,
) -> InvokeResponse<PluginPayload> {
    // 自动模式：无人值守，不产确认卡，直接返回友好错误让 LLM 继续（不阻塞）。
    // failure_kind=permission_denied 标记，前端可据此渲染（虽然不产节点，仅信息性）。
    if mode == "auto" {
        return Ok(PluginPayload::new(&json!({
            "error": format!(
                "权限不足：工具 {} 需要用户审批（风险等级 {}），但当前为自动模式，无人可授权。请勿反复重试——请改用手动方式完成，或提示用户切换到交互模式以授权后重试。",
                tool_name, risk_level
            ),
            "success": false,
            "failure_kind": "permission_denied",
        })));
    }
    let (tx_side, rx_side) = PluginChannel::pair(16);
    let node = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        parent_id: None,
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::UserPrompt),
        content: Some(MessageContent::Text(format!(
            "需要确认：{tool_description}"
        ))),
        status: Some(MessageStatus::WaitingUserAction),
        meta: Some(json!({
            "prompt": {
                "kind": "confirm",
                "tool_name": tool_name,
                "args": args.clone(),
                "risk_level": risk_level,
                "description": tool_description,
            },
            // failure_kind=needs_approval：前端据此渲染"批准 / 拒绝"按钮（与错误盒统一）
            "failure_kind": "needs_approval"
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
    drop(tx_side);
    Ok(PluginPayload::Session(rx_side))
}

pub struct SecureToolWrapper {
    inner: Arc<dyn Capability>,
    security: Arc<SecurityPolicy>,
}

impl SecureToolWrapper {
    pub fn new(inner: Arc<dyn Capability>, security: Arc<SecurityPolicy>) -> Self {
        Self { inner, security }
    }
}

#[async_trait]
impl Capability for SecureToolWrapper {
    fn meta(&self) -> CapabilityMeta {
        self.inner.meta()
    }

    fn name(&self) -> String {
        self.inner.name()
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
        let tool_name = self.inner.name();
        let tool_risk_level = self.security.get_tool_risk_level(&tool_name, Some(&args));

        let is_approved = args
            .get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // per-session 风险等级阈值：从 ctx[RISK_LEVEL] 读出（与 agent_id/provider_id/mode 同级别）
        let threshold = risk_level_from_ctx(&ctx);

        let (suggested_approval, final_risk_level) =
            self.security
                .check_tool_approval_needed(&tool_name, tool_risk_level, threshold);

        let needs_approval = suggested_approval && !is_approved;

        if needs_approval {
            // 产出 confirm 类型 user_prompt 节点（交互模式），或自动模式返回友好错误
            let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();
            return emit_confirm_prompt(
                &tool_name,
                &self.inner.meta().description,
                &args,
                &format!("{final_risk_level:?}").to_lowercase(),
                &mode,
            )
            .await;
        }

        self.inner.execute(ctx).await
    }
}

#[derive(Clone)]
pub struct LocalPlugin {
    config: Arc<RwLock<LocalConfig>>,
    tool_impls: Arc<Vec<Arc<dyn Capability>>>,
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
    security: Arc<SecurityPolicy>,
}

impl LocalPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let config: LocalConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let parent = ctx.parent();

        Arc::new(LocalPlugin::new(parent, config)) as Arc<dyn Plugin>
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: LocalConfig) -> Self {
        let security = Arc::new(SecurityPolicy::default());
        let config_lock = Arc::new(RwLock::new(config));

        let file_read = Arc::new(FileReadTool::new(Arc::clone(&security)));
        let file_write = Arc::new(FileWriteTool::new(Arc::clone(&security)));
        let file_edit = Arc::new(FileEditTool::new(Arc::clone(&security)));
        let shell = Arc::new(ShellTool::new(Arc::clone(&security)));
        let file_search = Arc::new(FileSearchTool::new(Arc::clone(&security)));
        let content_search = Arc::new(ContentSearchTool::new(Arc::clone(&security)));
        let dir_list = Arc::new(DirListTool::new(Arc::clone(&security)));
        let todo_write = Arc::new(TodoWriteTool::new(Arc::clone(&security)));
        let ask_user = Arc::new(AskUserTool::new(Arc::clone(&security)));
        let codebase_search = Arc::new(CodebaseSearchTool::new(Arc::clone(&security)));
        let file_delete = Arc::new(FileDeleteTool::new(Arc::clone(&security)));

        let tool_impls: Vec<Arc<dyn Capability>> = vec![
            file_read,
            file_edit,
            file_write,
            file_delete,
            shell,
            file_search,
            content_search,
            dir_list,
            todo_write,
            ask_user,
            codebase_search,
        ];

        Self {
            config: config_lock,
            tool_impls: Arc::new(tool_impls),
            parent: Arc::new(RwLock::new(parent)),
            security,
        }
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("local", "本地工具集")
            .with_description("提供文件操作、Shell 命令等本地相关工具")
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for LocalPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();

        if path.starts_with('/') {
            if let Some(parent) = self.get_parent().await {
                return parent.route(ctx).await;
            }
        }

        match path.as_str() {
            CONFIG_GET => {
                let cfg = self.config.read().await;
                Ok(PluginPayload::new(&*cfg))
            },
            CONFIG_SET => {
                let new_cfg: LocalConfig = ctx.payload()?;
                {
                    let mut cfg = self.config.write().await;
                    *cfg = new_cfg.clone();
                }
                if let Some(p) = self.get_parent().await {
                    let save_ctx = ctx.fork();
                    save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
                    let _ = p.route(save_ctx).await;
                }
                Ok(PluginPayload::new(&SimpleResponse::success()))
            },
            _ => {
                let payload = ctx.payload::<serde_json::Value>()?;
                if let Some(tool) = self.tool_impls.iter().find(|t| t.name() == path) {
                    let tool_risk_level = self.security.get_tool_risk_level(&path, Some(&payload));

                    let is_approved = payload
                        .get("approved")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    // per-session 风险等级阈值：从 ctx[RISK_LEVEL] 读出（与 agent_id/provider_id/mode 同级别）
                    let threshold = risk_level_from_ctx(&ctx);

                    let (suggested_approval, final_risk_level) = self
                        .security
                        .check_tool_approval_needed(&path, tool_risk_level, threshold);

                    let needs_approval = suggested_approval && !is_approved;

                    if needs_approval {
                        // 产出 confirm 类型 user_prompt 节点（交互模式），或自动模式返回友好错误
                        let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();
                        return emit_confirm_prompt(
                            &path,
                            "工具执行",
                            &payload,
                            &format!("{final_risk_level:?}").to_lowercase(),
                            &mode,
                        )
                        .await;
                    }

                    return tool.execute(ctx).await;
                }
                Err(PluginError::NotFound(format!("路径不存在: {path}")))
            },
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let sub_path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        if sub_path != crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Err(crate::symbio_core::PluginError::NotFound(format!(
                "未知遍历路径: {}",
                sub_path
            )));
        }

        if let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
            for tool in self.tool_impls.iter() {
                let wrapped = Arc::new(SecureToolWrapper::new(tool.clone(), self.security.clone()));
                tool_manager.register(wrapped).await;
            }
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_LOCAL, LocalPlugin::build, dyn Plugin);
