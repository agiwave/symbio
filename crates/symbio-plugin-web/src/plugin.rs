//! Web Tools 插件实现

pub use super::web_config::WebConfig;
use super::{http_request::HttpRequestTool, web_fetch::WebFetchTool, web_search::WebSearchTool};
// 审批闸门住在能力注册收口点（`providers/collectors/approval.rs`）；`route` 兜底
// 拿到的是本插件私藏的裸工具，判定与装饰器共用同一份真源（`http_request`
// 声明 `High` 却经这条路径直接跑，正是「风险只是宣传语」的第二个洞）。
use symbio_deploy::PLUGIN_ID_WEB;
use symbio_providers::approval_gate;
use symbio_core::{
    plugin_dir_from_ctx, Capability, CapabilityRow, Plugin, PluginConfigFile, PluginDir,
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta,
    PluginPayload,
};
use symbio_core::{DetailDefinition, DetailField};
use async_trait::async_trait;
use std::sync::{Arc, Weak};
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

#[derive(Clone)]
pub struct WebPlugin {
    config: Arc<RwLock<WebConfig>>,
    /// 配置文件的呈现与校验（`<根>/web/PLUGIN.yml`）——落盘写的是本插件自己目录里的文件
    config_file: PluginConfigFile,
    tool_impls: Arc<Vec<Arc<dyn Capability>>>,
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
                symbio_core::plugin_warn!("web", "读取自身配置失败，改用默认值：{e}");
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

        let tool_impls: Vec<Arc<dyn Capability>> = vec![web_fetch, web_search, http_request];

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

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn symbio_core::VdfsProvider>> {
        Some(self)
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(symbio_core::PATH).unwrap_or_default();

        if path.starts_with('/') {
            if let Some(parent) = self.get_parent().await {
                return parent.route(ctx).await;
            }
        }

        if let Some(tool) = self.tool_impls.iter().find(|t| t.name() == path) {
            let args = ctx
                .payload::<serde_json::Value>()
                .unwrap_or(serde_json::Value::Null);
            if let Some(payload) = approval_gate(&ctx, tool.as_ref(), &args)? {
                return Ok(PluginPayload::new(&payload));
            }
            return symbio_core::capability_invoke(tool.as_ref(), ctx).await;
        }
        Err(PluginError::NotFound(format!("路径不存在: {path}")))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(symbio_core::PATH).unwrap_or_default();
        if sub_path != symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Err(symbio_core::PluginError::NotFound(format!(
                "未知遍历路径: {}",
                sub_path
            )));
        }

        if let Some(visitor) = ctx.get(symbio_core::CAPABILITY_VISITOR) {
            for tool in self.tool_impls.iter() {
                visitor.put(CapabilityRow::tool(tool.clone())).await;
            }
        }
        // 顺带声明「本插件有一份配置文档」：设置页据此列出本项并指路到
        // `<根>/web/PLUGIN.yml`（标签与配置节点共用同一个来源，见 `PluginConfigFile`）
        symbio_core::capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`<根>/web/PLUGIN.yml`） ====================
//
// 本插件在 VDFS 上的全部内容 = 一个配置文档，故实现 [`PluginConfigMount`] 即可——
// 四臂 dispatch 由机制侧提供（见 `symbio_core::PluginConfigMount`），本处只声明差异。

impl symbio_core::PluginConfigMount for WebPlugin {
    type Config = WebConfig;
    const TITLE: &'static str = "网络工具";

    fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    fn config_slot(&self) -> &RwLock<WebConfig> {
        &self.config
    }
}

symbio_core::submit_object_creator!(PLUGIN_ID_WEB, WebPlugin::build, dyn Plugin);

symbio_core::impl_config_mount_vdfs!(WebPlugin);
