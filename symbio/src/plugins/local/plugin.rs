//! Local Tools 插件实现

pub use super::local_config::LocalConfig;
use super::policy::{RiskLevel, SecurityPolicy};
use super::{
    codebase_search::CodebaseSearchTool, content_search::ContentSearchTool, shell::ShellTool,
    todo_write::TodoWriteTool,
};
use crate::providers::vdfs_service::config::{self, ConfigDoc};
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::schemas::session::session_chat_response;
use crate::symbio_core::vdfs;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginChannel, PluginError, PluginFrame, PluginMeta, PluginPayload, PLUGIN_LOCAL,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

// ==================== 配置文档（`.vdfs/local/配置`） ====================

/// 本地工具配置的定义 —— **定义由配置的拥有者产出**。
///
/// 字段真源即 [`LocalConfig`]：默认值从 `Default` 读出，不写第二份字面量
/// （历史上定义寄居在 setting 插件里，schema 与 serde 各写一份默认值，
/// 出现过面板显示值与实际行为不符的漂移）。
fn config_definition() -> DetailDefinition {
    let d = LocalConfig::default();
    DetailDefinition::form(
        "本地工具设置",
        vec![
            DetailField::toggle(
                "shell_enabled",
                "启用 Shell 工具",
                "允许执行 Shell 命令",
                d.shell_enabled,
            ),
            DetailField::toggle(
                "file_enabled",
                "启用文件工具",
                "允许文件读写操作",
                d.file_enabled,
            ),
            DetailField::number(
                "shell_timeout",
                "Shell 超时（秒）",
                "Shell 命令执行超时时间",
                1.0,
                3600.0,
                json!(d.shell_timeout),
            ),
        ],
    )
}

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
    /// 配置文档（`.vdfs/local/配置`）——节点形状 / 校验 / 落盘推送给它
    config_doc: ConfigDoc,
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

        let shell = Arc::new(ShellTool::new(Arc::clone(&security)));
        let content_search = Arc::new(ContentSearchTool::new(Arc::clone(&security)));
        let todo_write = Arc::new(TodoWriteTool::new(Arc::clone(&security)));
        let codebase_search = Arc::new(CodebaseSearchTool::new(Arc::clone(&security)));

        // 文件编辑类能力（read/edit/write/delete/list/search）已迁入 VDFS 的物理层
        // （见 plugins/vdfs/physical.rs），由 `vdfs` 插件以 `vdfs_*` 工具统一暴露，
        // 此处不再提供原生工具。本插件在 VDFS 上只挂一个**配置文档**。
        let tool_impls: Vec<Arc<dyn Capability>> =
            vec![shell, content_search, todo_write, codebase_search];

        Self {
            config: config_lock,
            config_doc: ConfigDoc::new(PLUGIN_LOCAL, "本地工具", config_definition()),
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

        let payload = ctx.payload::<serde_json::Value>()?;
        if let Some(tool) = self.tool_impls.iter().find(|t| t.name() == path) {
            let tool_risk_level = self.security.get_tool_risk_level(&path, Some(&payload));

            let is_approved = payload
                .get("approved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // per-session 风险等级阈值：从 ctx[RISK_LEVEL] 读出（与 agent_id/provider_id/mode 同级别）
            let threshold = risk_level_from_ctx(&ctx);

            let (suggested_approval, final_risk_level) =
                self.security
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

        if let Some(visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
            for tool in self.tool_impls.iter() {
                let wrapped = Arc::new(SecureToolWrapper::new(tool.clone(), self.security.clone()));
                visitor.register(wrapped).await;
            }
            // 与工具共用同一次能力广播：本插件在 VDFS 上的全部内容 = 一个配置文档
            let me: vdfs::DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_LOCAL, me).await;
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`.vdfs/local/配置`） ====================
//
// 本插件只有配置、没有资源树，因此挂载根的内容恒为「一个配置文档」。
// 节点形状、定义校验、落盘推送都在 [`ConfigDoc`] 里，这里只做寻址分流。

#[async_trait]
impl vdfs::VdfsProvider for LocalPlugin {
    fn label(&self) -> Option<&str> {
        Some("本地工具")
    }

    fn description(&self) -> Option<&str> {
        Some("本地 Shell / 文件工具的配置。")
    }

    fn order(&self) -> i32 {
        7
    }

    fn icon(&self) -> Option<&str> {
        Some("terminal")
    }

    /// 根下只有配置文档，不接受新建 / 建目录
    fn root_access(&self) -> vdfs::VdfsAccess {
        vdfs::VdfsAccess::LIST
    }

    async fn list(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<Vec<vdfs::VdfsNode>> {
        if path.is_empty() {
            return Ok(vec![self.config_doc.node()]);
        }
        Err(vdfs::VdfsError::not_found(format!(
            "本地工具是配置挂载点，没有子项：{path}"
        )))
    }

    async fn stat(&self, _ctx: &vdfs::VdfsContext, path: &str) -> vdfs::VdfsResult<vdfs::VdfsNode> {
        if path.is_empty() {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            return Ok(vdfs::VdfsNode::dir("", "本地工具", self.root_access()));
        }
        if config::is_config_path(path) {
            return Ok(self.config_doc.node());
        }
        Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
    }

    async fn read(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<vdfs::VdfsContent> {
        if config::is_config_path(path) {
            return self.config_doc.read(&self.config).await;
        }
        Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
    }

    async fn write(
        &self,
        ctx: &vdfs::VdfsContext,
        path: &str,
        content: &vdfs::VdfsContent,
    ) -> vdfs::VdfsResult<vdfs::VdfsWriteResponse> {
        if config::is_config_path(path) {
            return self.config_doc.apply(ctx, &self.config, content).await;
        }
        Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
    }
}

crate::submit_object_creator!(PLUGIN_LOCAL, LocalPlugin::build, dyn Plugin);
