//! Web Tools 插件实现

pub use super::web_config::WebConfig;
use super::{http_request::HttpRequestTool, web_fetch::WebFetchTool, web_search::WebSearchTool};
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::vdfs;
use crate::symbio_core::{
    dir_from_ctx, Capability, ConfigFile, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginDir, PluginError, PluginMeta, PluginPayload, PLUGIN_FILE, PLUGIN_WEB,
};
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
            DetailField::toggle(
                "web_enabled",
                "启用 Web 工具",
                "允许网络请求",
                d.web_enabled,
            ),
            DetailField::number(
                "web_timeout",
                "Web 超时（秒）",
                "Web 请求超时时间",
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
        ],
    )
}

#[derive(Clone)]
pub struct WebPlugin {
    config: Arc<RwLock<WebConfig>>,
    /// 配置文件的呈现与校验（`<根>/web/PLUGIN.yml`）——落盘写的是本插件自己目录里的文件
    config_file: ConfigFile,
    tool_impls: Arc<Vec<Arc<dyn Capability>>>,
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl WebPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let dir = dir_from_ctx(&*ctx, PLUGIN_WEB);
        let config: WebConfig = match dir.load::<WebConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => WebConfig::default(),
            Err(e) => {
                crate::plugin_warn!("web", "读取自身配置失败，改用默认值：{e}");
                WebConfig::default()
            }
        };

        let parent = ctx.parent();

        Arc::new(WebPlugin::new(parent, config, dir)) as Arc<dyn Plugin>
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: WebConfig, dir: PluginDir) -> Self {
        let config_lock = Arc::new(RwLock::new(config));

        let web_fetch = Arc::new(WebFetchTool::new());
        let web_search = Arc::new(WebSearchTool::new(Arc::clone(&config_lock)));
        let http_request = Arc::new(HttpRequestTool::new());

        let tool_impls: Vec<Arc<dyn Capability>> = vec![web_fetch, web_search, http_request];

        Self {
            config: config_lock,
            config_file: ConfigFile::new(dir, "网络工具", config_definition()),
            tool_impls: Arc::new(tool_impls),
            parent: Arc::new(RwLock::new(parent)),
        }
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("web", "网络工具集")
            .with_description("提供 Web 搜索、网络请求等网络相关工具")
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for WebPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(
        self: Arc<Self>,
    ) -> Option<Arc<dyn crate::symbio_core::vdfs_provider::VdfsProvider>> {
        Some(self)
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();

        if path.starts_with('/') {
            if let Some(parent) = self.get_parent().await {
                return parent.route(ctx).await;
            }
        }

        if let Some(tool) = self.tool_impls.iter().find(|t| t.name() == path) {
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
                visitor.register(tool.clone()).await;
            }
            // 与工具共用同一次能力广播：本插件在 VDFS 上的全部内容 = 一个配置文档
            let me: vdfs::DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_WEB, me).await;
        }
        // 顺带声明「本插件有一份配置文档」：设置页据此列出本项并指路到
        // `<根>/web/PLUGIN.yml`（标签与配置节点共用同一个来源，见 `ConfigFile`）
        crate::symbio_core::announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`<根>/web/PLUGIN.yml`） ====================

#[async_trait]
impl vdfs::VdfsProvider for WebPlugin {
    fn label(&self) -> Option<&str> {
        Some("网络工具")
    }

    fn description(&self) -> Option<&str> {
        Some("Web 工具的启用、超时与搜索服务凭据。")
    }

    fn order(&self) -> i32 {
        8
    }

    fn icon(&self) -> Option<&str> {
        Some("globe")
    }

    /// **隐藏**：本挂载点的全部内容就是一份配置文档，没有用户资源可浏览，
    /// 所以它在父目录的列表里不出现（与文件 / 目录的隐藏属性同一件事）。
    /// 挂载本身照旧——按路径（`<根>/web/PLUGIN.yml`）仍完全可寻址。
    fn root_hidden(&self) -> bool {
        true
    }

    /// 根下只有配置文件，不接受新建 / 建目录
    fn root_access(&self) -> vdfs::VdfsAccess {
        vdfs::VdfsAccess::LIST
    }

    async fn list(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<Vec<vdfs::VdfsNode>> {
        if path.is_empty() {
            return Ok(vec![self.config_file.node()]);
        }
        Err(vdfs::VdfsError::not_found(format!(
            "网络工具是配置挂载点，没有子项：{path}"
        )))
    }

    async fn stat(&self, _ctx: &vdfs::VdfsContext, path: &str) -> vdfs::VdfsResult<vdfs::VdfsNode> {
        if path.is_empty() {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            return Ok(vdfs::VdfsNode::dir("", "网络工具", self.root_access()));
        }
        if path == PLUGIN_FILE {
            return Ok(self.config_file.node());
        }
        Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
    }

    async fn read(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<vdfs::VdfsContent> {
        if path == PLUGIN_FILE {
            return self.config_file.read(&self.config).await;
        }
        Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
    }

    async fn write(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        content: &vdfs::VdfsContent,
    ) -> vdfs::VdfsResult<vdfs::VdfsWriteResponse> {
        if path == PLUGIN_FILE {
            return self.config_file.apply(&self.config, content).await;
        }
        Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
    }
}

crate::submit_object_creator!(PLUGIN_WEB, WebPlugin::build, dyn Plugin);
