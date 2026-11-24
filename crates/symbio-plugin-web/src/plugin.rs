//! Web Tools 插件实现

use super::net_guard::normalized_url_fact;
pub use super::web_config::WebConfig;
use super::{
    http_request::HttpRequestTool, web_fetch::WebFetchTool, web_search::upstream_url,
    web_search::WebSearchTool,
};
// 审批闸门住在能力注册收口点（`providers/collectors/approval.rs`）；本插件只交声明，
// 不再有第二条路径去判闸（D-466）——`http_request` 声明 `High` 而「风险只是宣传语」
// 的第二个洞随 `route` 执行分支一起消失。
use async_trait::async_trait;
use std::sync::{Arc, Weak};
use symbio_plugin_api::PLUGIN_ID_WEB;
use symbio_plugin_api::{
    plugin_dir_from_ctx, CapabilityMeta, Plugin, PluginConfigFile, PluginDir, PluginError,
    PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload,
    PortExecutor,
};
use symbio_plugin_api::{DetailDefinition, DetailField};
use symbio_providers::FactsOf;
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

/// 注册用的工具条目：自述快照 + 端口执行体。
///
/// 工具改形为 [`PortExecutor`] 后不再有 `Arc<dyn Capability>` 可藏；`traverse` 把
/// 快照与执行体交给注册收口点（`register_tool`），闸门与行都在那里成形。本插件
/// **不执行**工具——`route` 对工具名一律 `NotFound`（D-466）。
///
/// 本插件全部工具都是**静态档**，故条目不持风险回答：动态覆写那一格随第二条执行面
/// 一起消失，`register_tool` 一律收 `risk = None`（判定回落 `meta.effective_risk()`）。
struct ToolEntry {
    meta: CapabilityMeta,
    facts: Option<FactsOf>,
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

        let url_facts: FactsOf = Arc::new(
            |args: &serde_json::Value, _ctx: &Arc<dyn symbio_plugin_api::PluginInvokeRequest>| {
                args.get("url")
                    .and_then(|value| value.as_str())
                    .map(|raw| {
                        vec![symbio_plugin_api::SandboxFact::new(
                            "url",
                            normalized_url_fact(raw),
                        )]
                    })
                    .unwrap_or_default()
            },
        );
        // FactsOf 是同步回调，不能用会在 async runtime 里 panic 的 blocking_read。
        // 正常路径现读当前配置；若正逢极短写锁窗口，退到稳定官方 endpoint，并共享实际 URL 投影。
        // WHY：回调无法 await，宁可交稳定 URL 让规则显式裁决，也不把模型 query 冒充上游。
        // 只提交实际 endpoint（DuckDuckGo 含查询参数），API key 留在 body/header，绝不进 URL fact。
        let search_config = Arc::clone(&config_lock);
        let search_facts: FactsOf = Arc::new(
            move |args: &serde_json::Value,
                  _ctx: &Arc<dyn symbio_plugin_api::PluginInvokeRequest>| {
                let upstream = match search_config.try_read() {
                    Ok(cfg) => upstream_url(&cfg, args),
                    Err(_) => upstream_url(&WebConfig::default(), args),
                };
                vec![symbio_plugin_api::SandboxFact::new(
                    "url",
                    normalized_url_fact(&upstream),
                )]
            },
        );
        // 三个工具都是**静态档**：风险交 `register_tool` 的 `None` 分支兜底。
        let tool_impls: Vec<ToolEntry> = vec![
            ToolEntry {
                meta: web_fetch.meta(),
                facts: Some(url_facts.clone()),
                executor: web_fetch,
            },
            ToolEntry {
                meta: web_search.meta(),
                facts: Some(search_facts),
                executor: web_search,
            },
            ToolEntry {
                meta: http_request.meta(),
                facts: Some(url_facts),
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
            .with_icon(r#"<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>"#)
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

        // 工具不再有第二条执行面（`D-440` / `D-466`）：执行只经 `port.tool` 契约，
        // 闸门在注册收口点的 `GatedExecutor` 里。按工具名 `route` 一律 `NotFound`——
        // 保留父容器转发，是为了让本插件仍能代理挂在它下面的地址面。
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

        if ctx.get(symbio_plugin_api::CAPABILITY_VISITOR).is_some() {
            // 注册收口点（`register_tool`）一次写两处：裹闸的端口绑定 + 纯数据的
            // 工具行。本插件全部工具都是静态档（`risk = None`）。
            for entry in self.tool_impls.iter() {
                symbio_providers::register_tool_with_facts(
                    &ctx,
                    entry.meta.clone(),
                    None,
                    entry.facts.clone(),
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

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;

symbio_plugin_api::impl_config_mount_vdfs!(WebPlugin);
