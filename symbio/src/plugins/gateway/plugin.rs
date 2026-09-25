//! 网关插件 —— 负责「本应用如何被调用」以及「本应用调用谁」
//!
//! 挂载于 worker composite 之下（与 `model`/`session` 平级），故 `ctx.parent()` 即为
//! worker，调用 `parent.route(ctx)` 等价于既有的 `root.route`：能转发前端会发起的全部路径
//!（`session/*`、`model/*`、`vdfs/*` …）。因此本插件**无需任何新全局注册表**。

use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField, DetailOption};
use crate::symbio_core::vdfs;
use crate::symbio_core::{
    dir_from_ctx, Plugin, PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload, PATH, PLUGIN_FILE,
    PLUGIN_GATEWAY,
};
use async_trait::async_trait;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::config::GatewayConfig;
use super::server;

// ==================== 配置文档（`<根>/gateway/PLUGIN.yml`） ====================

/// 网关配置的定义 —— **定义由配置的拥有者产出**。
///
/// 默认值从 [`GatewayConfig::default()`] 读出，不写第二份字面量。
fn config_definition() -> DetailDefinition {
    let d = GatewayConfig::default();
    DetailDefinition::form(
        "开放接口",
        vec![
            DetailField::toggle("inbound_enabled", "启用入站服务", "", d.inbound_enabled)
                .with_description(
                    "开启后本应用通过 HTTP/WebSocket 对外提供与前端完全一致的 API，\
                     第三方或其他 Symbio 实例可据此驱动本应用",
                ),
            DetailField::select(
                "inbound_protocol",
                "入站协议",
                vec![
                    DetailOption {
                        value: "native".into(),
                        label: "native（仅本机 Tauri IPC，不监听端口）".into(),
                        description: None,
                    },
                    DetailOption {
                        value: "http".into(),
                        label: "http（监听端口，第三方/其他实例可访问）".into(),
                        description: None,
                    },
                ],
                &d.inbound_protocol,
            ),
            DetailField::text(
                "inbound_bind",
                "监听地址",
                "127.0.0.1 仅本机；0.0.0.0 暴露给全网（需配合访问令牌）",
            ),
            DetailField::number(
                "inbound_port",
                "监听端口",
                "",
                1.0,
                65535.0,
                serde_json::json!(d.inbound_port),
            ),
            DetailField::password(
                "inbound_token",
                "访问令牌 (API Key)",
                "Bearer Token；回环地址可留空，非回环地址必填",
                "留空则不校验（仅限 127.0.0.1）",
            ),
            DetailField::toggle(
                "inbound_readonly",
                "只读模式",
                "仅放行查询类路径，禁止写操作与命令执行",
                d.inbound_readonly,
            ),
        ],
    )
}

pub struct GatewayPlugin {
    config: Arc<RwLock<GatewayConfig>>,
    /// 配置文件的呈现与校验（`<根>/gateway/PLUGIN.yml`）——落盘写的是自己目录里的文件
    config_file: PluginConfigFile,
    /// 父插件（worker composite）弱引用，用于转发请求
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
    /// 入站服务句柄（为空表示未启动）
    server: Arc<RwLock<Option<server::ServerHandle>>>,
}

impl GatewayPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例（submit_object_creator! 自注册）
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = dir_from_ctx(&*ctx, PLUGIN_GATEWAY);
        let config: GatewayConfig = match dir.load::<GatewayConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => GatewayConfig::default(),
            Err(e) => {
                crate::plugin_warn!("gateway", "读取自身配置失败，改用默认值：{e}");
                GatewayConfig::default()
            }
        };
        let parent = ctx.parent();
        let arc = Arc::new(Self::new(parent, config, dir));
        // 启动期若已启用入站服务，则拉起监听（配置变更后由 VDFS 写入路径重建）
        let arc2 = arc.clone();
        tokio::spawn(async move {
            arc2.start_server_logged().await;
        });
        arc
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: GatewayConfig, dir: PluginDir) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, "开放接口", config_definition()),
            parent: Arc::new(RwLock::new(parent)),
            server: Arc::new(RwLock::new(None)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_GATEWAY, "开放接口")
            .with_description("本应用如何被调用（入站，对外提供服务）。")
            .with_version("0.1.0")
            .with_order(9)
            .with_icon("plug")
            .with_hidden(true)
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.read().await.as_ref().and_then(|w| w.upgrade())
    }

    /// 按当前配置启动入站服务（仅 http 协议且 enabled 时监听端口）。
    /// 失败经返回值上交，由调用方决定呈现方式（CLI 无 tracing subscriber，
    /// `warn!` 在那里不可见——见 [`Self::start_server_logged`]）。
    pub async fn start_server(&self) -> Result<(), String> {
        let cfg = self.config.read().await.clone();
        if !cfg.inbound_enabled || cfg.inbound_protocol != "http" {
            return Ok(());
        }
        let Some(parent) = self.get_parent().await else {
            return Err("父插件不可用，无法启动入站服务".to_string());
        };
        match server::start(&cfg, parent).await {
            Ok(Some(handle)) => {
                info!(
                    addr = %format!("{}:{}", cfg.inbound_bind, cfg.inbound_port),
                    "[gateway] 入站服务已启动"
                );
                *self.server.write().await = Some(handle);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => {
                warn!(error = %e, "[gateway] 入站服务启动失败");
                Err(e)
            }
        }
    }

    /// 启动入站服务并让失败在**无 tracing subscriber** 的宿主（CLI）里也可见：
    /// 构造期 fire-and-forget spawn 里的错误原先只进 `warn!`，被静默丢弃——
    /// 端口被占/绑定失败时调用方只看到「网关没起来」却无从归因。
    pub async fn start_server_logged(&self) {
        if let Err(e) = self.start_server().await {
            crate::plugin_warn!("gateway", "入站服务启动失败：{e}");
        }
    }

    /// 停止入站服务
    pub async fn stop_server(&self) {
        if let Some(handle) = self.server.write().await.take() {
            server::stop(handle);
            info!("[gateway] 入站服务已停止");
        }
    }

    /// 应用配置变更：先停再启（端口/开关/协议变更必须重建监听）
    async fn apply_config(&self) {
        self.stop_server().await;
        self.start_server_logged().await;
    }
}

#[async_trait]
impl Plugin for GatewayPlugin {
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
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            "status" => {
                let cfg = self.config.read().await;
                let running = self.server.read().await.is_some();
                Ok(PluginPayload::new(&serde_json::json!({
                    "inbound_enabled": cfg.inbound_enabled,
                    "inbound_protocol": cfg.inbound_protocol,
                    "inbound_bind": cfg.inbound_bind,
                    "inbound_port": cfg.inbound_port,
                    "inbound_readonly": cfg.inbound_readonly,
                    "inbound_running": running,
                })))
            }
            _ => Err(PluginError::NotFound(format!("[gateway] 未知路径: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if let Some(visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
            // 本插件在 VDFS 上的全部内容 = 一个配置文档
            let me: vdfs::DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_GATEWAY, me).await;
        }
        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        crate::symbio_core::announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`<根>/gateway/PLUGIN.yml`） ====================

#[async_trait]
impl vdfs::VdfsProvider for GatewayPlugin {
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
                    "开放接口是配置挂载点，没有子项：{path}"
                )))
            }
            vdfs::VdfsRequest::Stat => {
                if path.is_empty() {
                    // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
                    return Ok(vdfs::VdfsResponse::Stat(vdfs::VdfsNode::dir(
                        "",
                        "开放接口",
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
                    self.apply_config().await;
                    return Ok(vdfs::VdfsResponse::Write(resp));
                }
                Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")))
            }
            _ => Err(vdfs::VdfsError::not_found(format!("未知路径：{path}"))),
        }
    }
}

crate::submit_object_creator!(PLUGIN_GATEWAY, GatewayPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
