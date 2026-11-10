//! Web Tools 插件实现

pub use super::web_config::WebConfig;
use super::{http_request::HttpRequestTool, web_fetch::WebFetchTool, web_search::WebSearchTool};
// 审批闸门住在能力注册收口点（`providers/collectors/approval.rs`）；`route` 兜底
// 拿到的是本插件私藏的裸执行体，判定与装饰器共用同一份真源（`http_request`
// 声明 `High` 却经这条路径直接跑，正是「风险只是宣传语」的第二个洞）。
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Weak};
use symbio_plugin_api::PLUGIN_ID_WEB;
use symbio_plugin_api::{
    plugin_dir_from_ctx, CapabilityMeta, CapabilityRiskLevel, ExecAbortSignal, ExecEventSink,
    Plugin, PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, PortExecContext, PortExecutor, PortInput,
    RESULT_KEY,
};
use symbio_plugin_api::{DetailDefinition, DetailField};
use symbio_providers::{approval_gate, RiskOf};
use tokio::sync::RwLock;

// ==================== 配置文档（`<根>/web/PLUGIN.yml`） ====================

/// 网络工具配置的定义 —— **定义由配置的拥有者产出**。
///
/// 默认值从 [`WebConfig::default()`] 读出，不写第二份字面量。
fn config_definition() -> DetailDefinition {
    let d = WebConfig::default();
    DetailDefinition::form(
        "网络工具设置",
        vec![
            DetailField::number(
                "web_timeout",
                "Web 超时（秒）",
                "单次网络请求超时（web_fetch、web_search 与 http_request 共用）",
                1.0,
                300.0,
                serde_json::json!(d.web_timeout),
            ),
            DetailField::password(
                "tavily_api_key",
                "Tavily API Key",
                "用于高级网页搜索（优先）",
                "输入 Tavily API Key",
            ),
            DetailField::password(
                "serper_api_key",
                "Serper API Key",
                "用于 Google 网页搜索（备用）",
                "输入 Serper API Key",
            ),
            DetailField::text(
                "search_api_base",
                "搜索上游基址",
                "留空 = 各供应商官方端点；e2e / 自建代理填基址（路径形状按供应商契约保留）",
            ),
            DetailField::list(
                "http_allowed_domains",
                "HTTP 出站域名白名单",
                "http_request 允许访问的域名（`*` = 不限制；`*.example.com` = 裸域名或子域）",
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
    /// 分支同义）；`Some` = 风险随参数变化的覆写闭包。
    risk: Option<RiskOf>,
    executor: Arc<dyn PortExecutor>,
}

#[derive(Clone)]
pub struct WebPlugin {
    config: Arc<RwLock<WebConfig>>,
    /// 配置文件的呈现与校验（`<根>/web/PLUGIN.yml`）——落盘写的是本插件自己目录里的文件
    config_file: PluginConfigFile,
    tool_impls: Arc<Vec<ToolEntry>>,
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl WebPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_WEB);
        let config: WebConfig = match dir.load::<WebConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => WebConfig::default(),
            Err(e) => {
                symbio_plugin_api::plugin_warn!("web", "读取自身配置失败，改用默认值：{e}");
                WebConfig::default()
            }
        };

        let parent = ctx.parent();

        Arc::new(WebPlugin::new(parent, config, dir)) as Arc<dyn Plugin>
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: WebConfig, dir: PluginDir) -> Self {
        // 三个工具共享同一份配置锁，每次调用现读：保存后立即生效，无需重载插件。
        // （构造期快照会让「面板上改了、工具还在用旧值」，且没有任何提示。）
        let config_lock = Arc::new(RwLock::new(config));

        let web_fetch = Arc::new(WebFetchTool::new(Arc::clone(&config_lock)));
        let web_search = Arc::new(WebSearchTool::new(Arc::clone(&config_lock)));
        let http_request = Arc::new(HttpRequestTool::new(Arc::clone(&config_lock)));

        // 三个工具都是**静态档**：风险交 `register_tool` 的 `None` 分支兜底，
        // 兜底条目在这里取好 meta 快照，两条路径判定一致。
        let tool_impls: Vec<ToolEntry> = vec![
            ToolEntry {
                meta: web_fetch.meta(),
                risk: None,
                executor: web_fetch,
            },
            ToolEntry {
                meta: web_search.meta(),
                risk: None,
                executor: web_search,
            },
            ToolEntry {
                meta: http_request.meta(),
                risk: None,
                executor: http_request,
            },
        ];

        Self {
            config: config_lock,
            config_file: PluginConfigFile::new(dir, "网络工具", config_definition()),
            tool_impls: Arc::new(tool_impls),
            parent: Arc::new(RwLock::new(parent)),
        }
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("web", "网络工具")
            .with_description("Web 工具的启用、超时与搜索服务凭据。")
            .with_version("0.1.0")
            .with_order(8)
            .with_icon("globe")
            .with_hidden(true)
    }
}

#[async_trait]
impl Plugin for WebPlugin {
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
            // 共用同一份真源 [`approval_gate`]。静态档工具取 `meta.effective_risk()`，
            // 与 `register_tool` 的 `risk = None` 分支同义。
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
            // 与执行体内部 `ExecEnv::from_request` 的读法同一口径；其余字段工具不读
            // （都从 ctx 取），填缺省值。
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
            // 工具行。本插件全部工具都是静态档（`risk = None`）。
            for entry in self.tool_impls.iter() {
                symbio_providers::register_tool(
                    visitor.as_ref(),
                    entry.meta.clone(),
                    None,
                    Arc::clone(&entry.executor),
                )
                .await;
            }
        }
        // 顺带声明「本插件有一份配置文档」：设置页据此列出本项并指路到
        // `<根>/web/PLUGIN.yml`（标签与配置节点共用同一个来源，见 `PluginConfigFile`）
        symbio_plugin_api::capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`<根>/web/PLUGIN.yml`） ====================
//
// 本插件在 VDFS 上的全部内容 = 一个配置文档，故实现 [`PluginConfigMount`] 即可——
// 四臂 dispatch 由机制侧提供（见 `symbio_plugin_api::PluginConfigMount`），本处只声明差异。

impl symbio_plugin_api::PluginConfigMount for WebPlugin {
    type Config = WebConfig;
    const TITLE: &'static str = "网络工具";

    fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    fn config_slot(&self) -> &RwLock<WebConfig> {
        &self.config
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_WEB, WebPlugin::build, dyn Plugin);

symbio_plugin_api::impl_config_mount_vdfs!(WebPlugin);
