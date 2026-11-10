//! Local Tools 插件实现

pub use super::local_config::LocalConfig;
use super::policy::{AutonomyLevel, SecurityPolicy};
use super::{
    ask_user::AskUserTool, codebase_search::CodebaseSearchTool, content_search::ContentSearchTool,
    shell::ShellTool, todo_write::TodoWriteTool,
};
// 审批闸门的判定住在能力注册收口点（`register_tool` 派生端口行时裹上的
// `GatedExecutor`）；本插件的 `route` 兜底也调它——两条路径**判定只有一份**。
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use symbio_plugin_api::PLUGIN_ID_LOCAL;
use symbio_plugin_api::{
    plugin_dir_from_ctx, CapabilityMeta, CapabilityRiskLevel, ExecAbortSignal, ExecEventSink,
    Plugin, PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, PortExecContext, PortExecutor, PortInput,
    RESULT_KEY,
};
use symbio_plugin_api::{DetailDefinition, DetailField, DetailOption};
use symbio_providers::{approval_gate, RiskOf};
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

/// `route` 兜底用的工具条目：自述快照 + 风险回答 + 端口执行体。
///
/// 工具改形为 [`PortExecutor`] 后不再有 `Arc<dyn Capability>` 可藏；兜底路径
/// 自己持有这三样，审批判定仍走 [`approval_gate`] 这一份真源，与注册收口点
/// （`register_tool`）同一读法。
struct ToolEntry {
    meta: CapabilityMeta,
    /// `None` = 静态档（`meta.effective_risk()`，与 `register_tool` 的 `risk = None`
    /// 分支同义）；`Some` = 风险随参数变化的覆写闭包（shell）。
    risk: Option<RiskOf>,
    executor: Arc<dyn PortExecutor>,
}

#[derive(Clone)]
pub struct LocalPlugin {
    config: Arc<RwLock<LocalConfig>>,
    /// 配置文件的呈现与校验（`<根>/local/PLUGIN.yml`）——节点形状 / 校验 / 落盘
    /// 都在它手上，落盘写的是**本插件自己目录里的**文件
    config_file: PluginConfigFile,
    tool_impls: Arc<Vec<ToolEntry>>,
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
                symbio_plugin_api::plugin_warn!("local", "读取自身配置失败，改用默认值：{e}");
                LocalConfig::default()
            }
        };

        let parent = ctx.parent();

        Arc::new(LocalPlugin::new(parent, config, dir)) as Arc<dyn Plugin>
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: LocalConfig, dir: PluginDir) -> Self {
        // 策略来自配置文档（默认全放开，见 PolicyRules::default 的说明）
        let security = Arc::new(SecurityPolicy::new(config.policy_rules()));
        // 读取 shell_timeout 配置（默认 60s，见 LocalConfig::default）
        let shell_timeout = config.shell_timeout;
        let config_lock = Arc::new(RwLock::new(config));
        let shell = Arc::new(ShellTool::new(Arc::clone(&security), shell_timeout));
        let content_search = Arc::new(ContentSearchTool::new(Arc::clone(&security)));
        // 下面两个**不持策略**（与 `ask_user` 同类的「没有可判的路径」）：审批闸门由
        // 收口点 `DefaultToolVisitor::put` 统一加壳（见 `providers/collectors/approval.rs`），
        // 理由见各自结构体文档。
        let todo_write = Arc::new(TodoWriteTool);
        let codebase_search = Arc::new(CodebaseSearchTool);
        // 询问用户：不接触文件系统，故不持 SecurityPolicy；产出 user_prompt 节点
        // 等用户回答（与下面的 confirm 审批共用同一套节点 / 回填机制）。
        let ask_user = Arc::new(AskUserTool);

        // 文件编辑类能力（read/edit/write/delete/list/search）已迁入 VDFS 的物理层
        // （见 crates/symbio-plugin-vdfs/src/physical.rs），由 `vdfs` 插件以 `vdfs_*` 工具统一暴露，
        // 此处不再提供原生工具。本插件在 VDFS 上只挂一个**配置文件**。
        //
        // shell 是唯一的**动态风险**工具：风险随命令而变（`git status` 是 Low，
        // `rm -rf ./` 是 High）。闭包捕获**同一份** `SecurityPolicy`，判定复用执行期
        // 同一个 [`SecurityPolicy::command_risk_level`]——审批卡上写的档与真正执行时
        // 判的档必然一致；闸门自己算一遍的话，两处规则会各自漂移。其余工具静态档
        // （`risk = None`，交 `register_tool` 读 `meta.effective_risk()`）。
        let shell_risk: RiskOf = {
            let security = Arc::clone(&security);
            Arc::new(move |args: &Value| {
                args.get("command")
                    .and_then(|v| v.as_str())
                    .map(|cmd| security.command_risk_level(cmd))
                    // 参数缺失/非字符串：执行期会以 ValidationError 拒掉，这里按最高档算
                    .unwrap_or(CapabilityRiskLevel::High)
            })
        };
        let tool_impls: Vec<ToolEntry> = vec![
            ToolEntry {
                meta: shell.meta(),
                risk: Some(shell_risk),
                executor: shell,
            },
            ToolEntry {
                meta: content_search.meta(),
                risk: None,
                executor: content_search,
            },
            ToolEntry {
                meta: todo_write.meta(),
                risk: None,
                executor: todo_write,
            },
            ToolEntry {
                meta: codebase_search.meta(),
                risk: None,
                executor: codebase_search,
            },
            ToolEntry {
                meta: ask_user.meta(),
                risk: None,
                executor: ask_user,
            },
        ];

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

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn symbio_plugin_api::VdfsProvider>> {
        Some(self)
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(symbio_plugin_api::PATH).unwrap_or_default();

        if path.starts_with('/') {
            if let Some(parent) = self.get_parent().await {
                return parent.route(ctx).await;
            }
        }

        if let Some(entry) = self.tool_impls.iter().find(|e| e.meta.name == path) {
            let args = ctx.payload::<Value>().unwrap_or(Value::Null);

            // 审批闸门：本处拿到的是**未包壳**的裸执行体（闸门长在注册收口点的
            // `GatedExecutor` 里），判定只能在这里自己走一遍——但判定与执行体
            // 共用同一份真源 [`approval_gate`]；shell 的动态风险覆写在闭包里原样生效。
            let static_risk = entry.meta.effective_risk();
            let static_risk_fn = |_: &Value| static_risk;
            let risk_fn: &dyn Fn(&Value) -> CapabilityRiskLevel = match &entry.risk {
                Some(risk) => risk.as_ref(),
                None => &static_risk_fn,
            };
            if let Some(payload) = approval_gate(&ctx, &entry.meta, risk_fn, &args) {
                return Ok(PluginPayload::new(&payload));
            }

            // 造最小端口上下文：sink / abort 从 ctx 读出（缺席即静默 / 永不中止），
            // 与执行体内部 `ExecEnv::from_request` 的读法同一口径——「会话里跑」与
            // 「被 route() 直连调用」仍共用同一条执行路径；其余字段工具不读（都从
            // ctx 取），填缺省值。
            let exec = PortExecContext {
                abort: ExecAbortSignal::of(&*ctx),
                idempotency_key: String::new(),
                sink: ExecEventSink::of(&*ctx),
                emit: symbio_plugin_api::port::PortFactEmit::silent(),
                ctx: Some(Arc::clone(&ctx)),
                subject: String::new(),
                group: String::new(),
                session_id: String::new(),
                origin: None,
                scratch_dir: None,
                reader: None,
                source: String::new(),
            };
            let outcome = entry
                .executor
                .execute(
                    exec,
                    PortInput {
                        port: path.clone(),
                        payload: args,
                    },
                )
                .await;
            // 信封 ↔ 结果换算：成功值在 `facts` 的 [`RESULT_KEY`] 项里（与派发侧
            // 同一条取法），失败转插件错误。
            return match outcome.error {
                Some((_, why)) => Err(PluginError::InternalError(why)),
                None => {
                    let value = outcome
                        .facts
                        .first()
                        .and_then(|f| f.get(RESULT_KEY))
                        .cloned()
                        .unwrap_or(Value::Null);
                    Ok(PluginPayload::new(&value))
                }
            };
        }
        Err(PluginError::NotFound(format!("路径不存在: {path}")))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(symbio_plugin_api::PATH).unwrap_or_default();
        if sub_path != symbio_plugin_api::TRAVERSE_AVAILABLE_TOOLS {
            return Err(symbio_plugin_api::PluginError::NotFound(format!(
                "未知遍历路径: {}",
                sub_path
            )));
        }

        if let Some(visitor) = ctx.get(symbio_plugin_api::CAPABILITY_VISITOR) {
            // 注册收口点（`register_tool`）一次写两条行：裹闸的端口行 + 纯数据的
            // 工具行。shell 的动态风险闭包随条目交上去，静态档工具传 `None`
            // （判定回落 `meta.effective_risk()`）。
            for entry in self.tool_impls.iter() {
                symbio_providers::register_tool(
                    visitor.as_ref(),
                    entry.meta.clone(),
                    entry.risk.clone(),
                    Arc::clone(&entry.executor),
                )
                .await;
            }
        }
        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        symbio_plugin_api::capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`<根>/local/PLUGIN.yml`） ====================
//
// 本插件只有配置、没有资源树，因此挂载根的内容恒为「一个配置文件」。
// 节点形状、定义校验、落盘都在 [`PluginConfigFile`] 里；四臂寻址分流由机制侧
// 提供（见 `symbio_plugin_api::PluginConfigMount`），本处只声明差异与写后副作用。
// 地址就是**真实文件名** `PLUGIN.yml`——它是插件目录里的一个普通文件。

#[async_trait]
impl symbio_plugin_api::PluginConfigMount for LocalPlugin {
    type Config = super::local_config::LocalConfig;
    const TITLE: &'static str = "本地工具";

    fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    fn config_slot(&self) -> &RwLock<Self::Config> {
        &self.config
    }

    /// 策略热更：`apply` 已把新配置写进 slot，这里同步刷到运行中的
    /// `SecurityPolicy`（限流 / 白名单 / 审批开关即时生效，无需重启）。
    async fn after_write(&self) {
        self.security
            .update_rules(self.config.read().await.policy_rules());
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_LOCAL, LocalPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;

symbio_plugin_api::impl_config_mount_vdfs!(LocalPlugin);
