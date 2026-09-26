//! Local Tools 插件实现

pub use super::local_config::LocalConfig;
use super::policy::{AutonomyLevel, RiskLevel, SecurityPolicy};
use super::{
    ask_user::AskUserTool, codebase_search::CodebaseSearchTool, content_search::ContentSearchTool,
    shell::ShellTool, todo_write::TodoWriteTool,
};
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField, DetailOption};
use crate::symbio_core::vdfs;
use crate::symbio_core::{
    plugin_dir_from_ctx, Capability, CapabilityMeta, ExecEnv, Plugin, PluginConfigFile, PluginDir,
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta,
    PluginPayload, PLUGIN_FILE, PLUGIN_ID_LOCAL,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

// ==================== 配置文档（`<根>/local/PLUGIN.yml`） ====================

/// 本地工具配置的定义 —— **定义由配置的拥有者产出**。
///
/// 字段真源即 [`LocalConfig`]：默认值从 `Default` 读出，不写第二份字面量
/// （历史上定义寄居在 plugin_manager 插件里，schema 与 serde 各写一份默认值，
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
            DetailField::select(
                "autonomy",
                "自主级别",
                vec![
                    DetailOption {
                        value: "readonly".into(),
                        label: "只读（禁止一切命令）".into(),
                        description: None,
                    },
                    DetailOption {
                        value: "supervised".into(),
                        label: "监督（危险操作需批准）".into(),
                        description: None,
                    },
                    DetailOption {
                        value: "full".into(),
                        label: "完全自主".into(),
                        description: None,
                    },
                ],
                match d.autonomy {
                    AutonomyLevel::ReadOnly => "readonly",
                    AutonomyLevel::Supervised => "supervised",
                    AutonomyLevel::Full => "full",
                },
            ),
            DetailField::number(
                "max_actions_per_hour",
                "每小时动作上限",
                "Shell 调用频次上限，0 = 不限流",
                0.0,
                1_000_000.0,
                json!(d.max_actions_per_hour),
            ),
            DetailField::toggle(
                "require_approval_for_medium_risk",
                "中风险命令需审批",
                "监督模式下中风险命令（mkdir/mv/cp 等）需用户批准",
                d.require_approval_for_medium_risk,
            ),
            DetailField::toggle(
                "block_high_risk_commands",
                "阻止高风险命令",
                "直接拒绝高风险命令（rm/shutdown 等），不提供审批机会",
                d.block_high_risk_commands,
            ),
            DetailField::toggle(
                "workspace_only",
                "限制读取在工作区内",
                "开启后绝对路径读取仅限工作区与下方白名单根目录",
                d.workspace_only,
            ),
            DetailField::list(
                "allowed_commands",
                "命令白名单",
                "每行一个命令名（支持 npm.cmd / python.exe 形态）；留空 = 不限制",
            ),
            DetailField::list(
                "forbidden_paths",
                "路径黑名单",
                "每行一个路径，支持 ~ 展开；留空 = 不限制",
            ),
            DetailField::list(
                "allowed_roots",
                "额外允许的读取根目录",
                "每行一个绝对路径，仅在「限制读取在工作区内」开启时生效",
            ),
        ],
    )
}

/// 从 `ctx[RISK_LEVEL]` 读取 per-session 风险等级阈值。
///
/// 与 agent_id/provider_id/mode 同级别：随 chat_send 传输，由 orchestrator 写入 ctx。
/// ctx 无值时默认 `Medium`（新会话尚未设置时的安全默认值）。
fn risk_level_from_ctx(ctx: &Arc<dyn PluginInvokeRequest>) -> RiskLevel {
    ctx.get(crate::symbio_core::RISK_LEVEL)
        .map(|s| match s.as_str() {
            "low" => RiskLevel::Low,
            "high" => RiskLevel::High,
            _ => RiskLevel::Medium,
        })
        .unwrap_or(RiskLevel::Medium)
}

/// 构造 confirm 类型 prompt 的**返回值**（不构造节点、不发事件）。
///
/// 职责边界：工具只回答「需要用户确认什么」——`prompt` 载荷 + `failure_kind`。
/// **user_prompt 节点由编排层构造**（`session/tool_executor.rs`）：它才拥有
/// `result_msg_id`（节点身份）与父 ToolCall 的终态，是「工具结果节点」的唯一写入者。
///
/// 历史上这里是「建通道 → 发一个 Upsert 帧 → drop tx → 返回 rx」，由消费方
/// 解回来、改 id、重新播一遍——于是同一个逻辑节点在前端有两个 id（原 id 与
/// `result_msg_id`），审批 UI 重复、resume 只删得掉一个。现在这条路径不存在了。
fn confirm_prompt_payload(
    tool_name: &str,
    tool_description: &str,
    args: &Value,
    risk_level: &str,
    mode: &str,
) -> Result<Value, PluginError> {
    // 自动模式：无人值守，不产确认卡，直接返回友好错误让 LLM 继续（不阻塞）。
    // failure_kind=permission_denied 标记，前端可据此渲染（虽然不产节点，仅信息性）。
    if mode == "auto" {
        return Ok(json!({
            "error": format!(
                "权限不足：工具 {} 需要用户审批（风险等级 {}），但当前为自动模式，无人可授权。请勿反复重试——请改用手动方式完成，或提示用户切换到交互模式以授权后重试。",
                tool_name, risk_level
            ),
            "success": false,
            "failure_kind": crate::symbio_core::failure_kind::PERMISSION_DENIED,
        }));
    }
    Ok(json!({
        "content": format!("需要确认：{tool_description}"),
        "success": false,
        // 编排层凭此标记把本轮收口为「等待用户动作」并构造 user_prompt 节点。
        "failure_kind": crate::symbio_core::failure_kind::NEEDS_APPROVAL,
        "prompt": {
            "kind": "confirm",
            "tool_name": tool_name,
            "args": args.clone(),
            "risk_level": risk_level,
            "description": tool_description,
        },
    }))
}

pub struct SecureToolWrapper {
    inner: Arc<dyn Capability>,
    security: Arc<SecurityPolicy>,
}

/// 审批闸门：判定本次工具调用是否需要用户审批，需要则给出**载荷**。
///
/// 返回 `Ok(Some(payload))` ⇒ 本次调用不执行，等用户审批；`Ok(None)` ⇒ 放行。
///
/// ## 为什么收成一处
///
/// 同一套判定曾以两份几乎逐字相同的代码存在：
/// [`LocalPlugin::route`]（工具经插件路由进来）与
/// [`SecureToolWrapper::execute`]（工具经能力注册表进来）各写一份。
///
/// 两条入口都**真实存在**——`route` 是注册表未命中时的回落路径，`wrapper` 是
/// 注册表路径——所以闸门确实要在两处把守，但**判定逻辑**只该有一份。
/// 两份并存的漂移不是假设，已经发生了：`description` 不一致（`route` 传
/// 字面量 `"工具执行"`，`wrapper` 传工具真实描述），同一个工具走不同入口，
/// 用户看到的审批卡文案不一样。
///
/// 因此：**闸门判两次，判定写一次**。
fn approval_gate(
    security: &SecurityPolicy,
    ctx: &Arc<dyn PluginInvokeRequest>,
    tool_name: &str,
    tool_description: &str,
    args: &Value,
) -> Result<Option<Value>, PluginError> {
    let tool_risk_level = security.get_tool_risk_level(tool_name, Some(args));

    // 用户已在审批卡上批准 → 参数里带 `approved: true`，本轮直接放行。
    let is_approved = args
        .get("approved")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // per-session 风险等级阈值：从 ctx[RISK_LEVEL] 读出（与 agent_id/provider_id/mode 同级别）
    let threshold = risk_level_from_ctx(ctx);

    let (suggested_approval, final_risk_level) =
        security.check_tool_approval_needed(tool_name, tool_risk_level, threshold);

    // 放行条件：策略没建议审批，或本次调用已带过批准
    if !suggested_approval || is_approved {
        return Ok(None);
    }

    // 产出 confirm 类型 user_prompt 节点（交互模式），或自动模式返回友好错误
    let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();
    confirm_prompt_payload(
        tool_name,
        tool_description,
        args,
        &format!("{final_risk_level:?}").to_lowercase(),
        &mode,
    )
    .map(Some)
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

    async fn execute(
        &self,
        args: Value,
        env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let meta = self.inner.meta();
        if let Some(payload) =
            approval_gate(&self.security, &ctx, &meta.name, &meta.description, &args)?
        {
            return Ok(payload);
        }

        // 装饰器只加一道审批闸门，执行期环境与信封原样透传。
        self.inner.execute(args, env, ctx).await
    }
}

#[derive(Clone)]
pub struct LocalPlugin {
    config: Arc<RwLock<LocalConfig>>,
    /// 配置文件的呈现与校验（`<根>/local/PLUGIN.yml`）——节点形状 / 校验 / 落盘
    /// 都在它手上，落盘写的是**本插件自己目录里的**文件
    config_file: PluginConfigFile,
    tool_impls: Arc<Vec<Arc<dyn Capability>>>,
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
    security: Arc<SecurityPolicy>,
}

impl LocalPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        // 自己的目录由容器经 `PLUGIN_DIR` 告知；配置就存在那里的 PLUGIN.yml
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_LOCAL);
        let config: LocalConfig = match dir.load::<LocalConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => LocalConfig::default(),
            Err(e) => {
                crate::plugin_warn!("local", "读取自身配置失败，改用默认值：{e}");
                LocalConfig::default()
            }
        };

        let parent = ctx.parent();

        Arc::new(LocalPlugin::new(parent, config, dir)) as Arc<dyn Plugin>
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: LocalConfig, dir: PluginDir) -> Self {
        // 策略来自配置文档（默认全放开，见 PolicyRules::default 的说明）
        let security = Arc::new(SecurityPolicy::new(config.policy_rules()));
        let config_lock = Arc::new(RwLock::new(config));

        let shell = Arc::new(ShellTool::new(Arc::clone(&security)));
        let content_search = Arc::new(ContentSearchTool::new(Arc::clone(&security)));
        let todo_write = Arc::new(TodoWriteTool::new(Arc::clone(&security)));
        let codebase_search = Arc::new(CodebaseSearchTool::new(Arc::clone(&security)));
        // 询问用户：不接触文件系统，故不持 SecurityPolicy；产出 user_prompt 节点
        // 等用户回答（与下面的 confirm 审批共用同一套节点 / 回填机制）。
        let ask_user = Arc::new(AskUserTool);

        // 文件编辑类能力（read/edit/write/delete/list/search）已迁入 VDFS 的物理层
        // （见 plugins/vdfs/physical.rs），由 `vdfs` 插件以 `vdfs_*` 工具统一暴露，
        // 此处不再提供原生工具。本插件在 VDFS 上只挂一个**配置文件**。
        let tool_impls: Vec<Arc<dyn Capability>> =
            vec![shell, content_search, todo_write, codebase_search, ask_user];

        Self {
            config: config_lock,
            config_file: PluginConfigFile::new(dir, "本地工具", config_definition()),
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
        PluginMeta::new("local", "本地工具")
            .with_description("本地 Shell / 文件工具的配置。")
            .with_version("0.1.0")
            .with_order(7)
            .with_icon("terminal")
            .with_hidden(true)
    }
}

#[async_trait]
impl Plugin for LocalPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn crate::symbio_core::VdfsProvider>> {
        Some(self)
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();

        if path.starts_with('/') {
            if let Some(parent) = self.get_parent().await {
                return parent.route(ctx).await;
            }
        }

        let payload = ctx.payload::<serde_json::Value>()?;
        if let Some(tool) = self.tool_impls.iter().find(|t| t.name() == path) {
            // 审批闸门与 `SecureToolWrapper::execute` 共用同一份判定
            // （见 [`approval_gate`]）——两条入口都真实存在，但判定只写一次。
            // 描述取**工具真实描述**，与 wrapper 路径一致：此前这里传字面量
            // `"工具执行"`，同一个工具走不同入口会得到不同的审批卡文案。
            let meta = tool.meta();
            if let Some(payload) =
                approval_gate(&self.security, &ctx, &path, &meta.description, &payload)?
            {
                return Ok(PluginPayload::new(&payload));
            }

            // 信封 ↔ 结果换算收口在 `capability_invoke`：本处与
            // `CapabilityVisitor::invoke` 走同一条拆信封路径。
            return crate::symbio_core::capability_invoke(tool.as_ref(), ctx).await;
        }
        Err(PluginError::NotFound(format!("路径不存在: {path}")))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
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
            visitor.register_vdfs_provider(PLUGIN_ID_LOCAL, me).await;
        }
        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        crate::symbio_core::capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`<根>/local/PLUGIN.yml`） ====================
//
// 本插件只有配置、没有资源树，因此挂载根的内容恒为「一个配置文件」。
// 节点形状、定义校验、落盘都在 [`PluginConfigFile`] 里，这里只做寻址分流。
// 地址就是**真实文件名** `PLUGIN.yml`——它是插件目录里的一个普通文件。

#[async_trait]
impl vdfs::VdfsProvider for LocalPlugin {
    async fn dispatch(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => {
                if path.is_empty() {
                    return Ok(vdfs::VdfsResponse::list(vec![self.config_file.node()]));
                }
                Err(vdfs::VdfsError::not_found(format!(
                    "本地工具是配置挂载点，没有子项：{path}"
                )))
            }
            vdfs::VdfsRequest::Stat => {
                if path.is_empty() {
                    // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
                    return Ok(vdfs::VdfsResponse::Stat(vdfs::VdfsNode::dir(
                        "",
                        "本地工具",
                        vdfs::VdfsAccess::LIST,
                    )));
                }
                if path == PLUGIN_FILE {
                    return Ok(vdfs::VdfsResponse::Stat(self.config_file.node()));
                }
                Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
            }
            vdfs::VdfsRequest::Read => {
                if path == PLUGIN_FILE {
                    return Ok(vdfs::VdfsResponse::Read(
                        self.config_file.read(&self.config).await?,
                    ));
                }
                Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
            }
            vdfs::VdfsRequest::Write { content } => {
                if path == PLUGIN_FILE {
                    let resp = self.config_file.apply(&self.config, &content).await?;
                    // 策略热更：apply 已把新配置写进 slot，同步刷到运行中的
                    // SecurityPolicy（限流 / 白名单 / 审批开关即时生效，无需重启）
                    self.security
                        .update_rules(self.config.read().await.policy_rules());
                    return Ok(vdfs::VdfsResponse::Write(resp));
                }
                Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
            }
            _ => Err(vdfs::VdfsError::not_found(format!("未知路径：{path}"))),
        }
    }
}

crate::submit_object_creator!(PLUGIN_ID_LOCAL, LocalPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
