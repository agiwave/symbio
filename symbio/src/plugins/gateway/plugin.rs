//! 网关插件 —— 负责「本应用如何被调用」以及「本应用调用谁」
//!
//! 挂载于 worker composite 之下（与 `model`/`session` 平级），故 `ctx.parent()` 即为
//! worker，调用 `parent.route(ctx)` 等价于既有的 `root.route`：能转发前端会发起的全部路径
//!（`session/*`、`model/*`、`vdfs/*` …）。因此本插件**无需任何新全局注册表**。

use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField, DetailOption};
use crate::symbio_core::vdfs;
use crate::symbio_core::{
    dir_from_ctx, ConfigFile, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginDir,
    PluginError, PluginMeta, PluginPayload, PATH, PLUGIN_FILE, PLUGIN_GATEWAY,
};
use async_trait::async_trait;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::config::GatewayConfig;
use super::server;

// ==================== 配置文档（`.vdfs/gateway/PLUGIN.yml`） ====================

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
                    },
                    DetailOption {
                        value: "http".into(),
                        label: "http（监听端口，第三方/其他实例可访问）".into(),
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
    /// 配置文件的呈现与校验（`.vdfs/gateway/PLUGIN.yml`）——落盘写的是自己目录里的文件
    config_file: ConfigFile,
    /// 父插件（worker composite）弱引用，用于转发请求
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
    /// 入站服务句柄（为空表示未启动）
    server: Arc<RwLock<Option<server::ServerHandle>>>,
}

impl GatewayPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例（submit_object_creator! 自注册）
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
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
            arc2.start_server().await;
        });
        arc
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: GatewayConfig, dir: PluginDir) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: ConfigFile::new(dir, "开放接口", config_definition()),
            parent: Arc::new(RwLock::new(parent)),
            server: Arc::new(RwLock::new(None)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_GATEWAY, "网关")
            .with_description("对外服务（入站）与连接方式（出站）的协议与参数配置")
            .with_version("0.1.0")
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.read().await.as_ref().and_then(|w| w.upgrade())
    }

    /// 按当前配置启动入站服务（仅 http 协议且 enabled 时监听端口）
    pub async fn start_server(&self) {
        let cfg = self.config.read().await.clone();
        if !cfg.inbound_enabled || cfg.inbound_protocol != "http" {
            return;
        }
        let Some(parent) = self.get_parent().await else {
            warn!("[gateway] 父插件不可用，无法启动入站服务");
            return;
        };
        match server::start(&cfg, parent).await {
            Ok(Some(handle)) => {
                info!(
                    addr = %format!("{}:{}", cfg.inbound_bind, cfg.inbound_port),
                    "[gateway] 入站服务已启动"
                );
                *self.server.write().await = Some(handle);
            }
            Ok(None) => {}
            Err(e) => warn!(error = %e, "[gateway] 入站服务启动失败"),
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
        self.start_server().await;
    }
}

#[async_trait]
impl Plugin for GatewayPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
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
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
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

// ==================== VDFS：配置文档（`.vdfs/gateway/PLUGIN.yml`） ====================

#[async_trait]
impl vdfs::VdfsProvider for GatewayPlugin {
    fn label(&self) -> Option<&str> {
        Some("开放接口")
    }

    fn description(&self) -> Option<&str> {
        Some("本应用如何被调用（入站，对外提供服务）。")
    }

    fn order(&self) -> i32 {
        9
    }

    fn icon(&self) -> Option<&str> {
        Some("plug")
    }

    /// **隐藏**：本挂载点的全部内容就是一份配置文档，没有用户资源可浏览，
    /// 所以它在父目录的列表里不出现（与文件 / 目录的隐藏属性同一件事）。
    /// 挂载本身照旧——按路径（`.vdfs/gateway/PLUGIN.yml`）仍完全可寻址。
    fn root_hidden(&self) -> bool {
        true
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
            return Ok(vec![self.config_file.node()]);
        }
        Err(vdfs::VdfsError::not_found(format!(
            "开放接口是配置挂载点，没有子项：{path}"
        )))
    }

    async fn stat(&self, _ctx: &vdfs::VdfsContext, path: &str) -> vdfs::VdfsResult<vdfs::VdfsNode> {
        if path.is_empty() {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            return Ok(vdfs::VdfsNode::dir("", "开放接口", self.root_access()));
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

    /// 写入：校验 → 落内存 → 落自己的文件 → 广播，随后**重建监听**
    /// （端口 / 开关 / 协议变更必须重建，这是本插件专有的副作用）
    async fn write(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        content: &vdfs::VdfsContent,
    ) -> vdfs::VdfsResult<vdfs::VdfsWriteResponse> {
        if path != PLUGIN_FILE {
            return Err(vdfs::VdfsError::not_found(format!("未知路径：{path}")));
        }
        let resp = self.config_file.apply(&self.config, content).await?;
        self.apply_config().await;
        Ok(resp)
    }
}

crate::submit_object_creator!(PLUGIN_GATEWAY, GatewayPlugin::build, dyn Plugin);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::vdfs::{VdfsContext, VdfsError, VdfsProvider};
    use crate::symbio_core::{
        InvokeRequestExt, InvokeResponse, Plugin, PluginPayload, SimpleRequest,
    };

    /// 以子插件直接收的**已剥离前缀**路径（如 `status`）调用 route，
    /// 模拟 home composite 转发后的行为。
    async fn call(
        plugin: Arc<GatewayPlugin>,
        path: &str,
        payload: Option<serde_json::Value>,
    ) -> InvokeResponse<PluginPayload> {
        let ctx = SimpleRequest::new(None, None);
        ctx.set(PATH, path.to_string());
        if let Some(p) = payload {
            ctx.set_payload(p).unwrap();
        }
        plugin.clone().route(Arc::new(ctx)).await
    }

    fn vctx() -> VdfsContext {
        VdfsContext::empty()
    }

    /// 测试用目录：这些用例都不触发落盘（坏值在写盘之前就被拦下），
    /// 因此指向一个临时目录即可，不必真建
    fn tdir() -> PluginDir {
        PluginDir::at(std::env::temp_dir().join("symbio-test-gateway"), "gateway")
    }

    /// 配置文档的形状：根下唯一一项、`ext = form`（前端据此选通用表单渲染器）、`rw`
    #[tokio::test]
    async fn config_document_is_the_only_child_of_the_root() {
        let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default(), tdir()));
        let items = plugin.list(&vctx(), "").await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, PLUGIN_FILE, "地址就是插件目录里的真实文件名");
        assert_eq!(items[0].title, "开放接口");
        assert_eq!(items[0].ext.as_deref(), Some(vdfs::VFDS_EXT_FORM));
        assert_eq!(items[0].access.flags(), "rw");
        assert!(items[0].schema.is_some(), "定义随节点下发");
    }

    /// 读：配置文档回当前配置（pretty JSON）
    #[tokio::test]
    async fn config_document_reads_current_config() {
        let cfg = GatewayConfig {
            inbound_enabled: true,
            inbound_protocol: "http".into(),
            inbound_port: 9231,
            ..GatewayConfig::default()
        };
        let plugin = Arc::new(GatewayPlugin::new(None, cfg, tdir()));
        let content = plugin.read(&vctx(), PLUGIN_FILE).await.unwrap();
        let got: GatewayConfig = serde_json::from_str(content.text.as_deref().unwrap()).unwrap();
        assert!(got.inbound_enabled);
        assert_eq!(got.inbound_protocol, "http");
        assert_eq!(got.inbound_port, 9231);
    }

    /// 写：校验先于一切——坏值在落内存之前就被定义拦下（字段级错误）
    #[tokio::test]
    async fn config_document_write_validates_before_applying() {
        let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default(), tdir()));
        let bad = vdfs::VdfsContent::text("", r#"{"inbound_port": 70000}"#);
        match plugin.write(&vctx(), PLUGIN_FILE, &bad).await {
            Err(VdfsError::Invalid(v)) => assert_eq!(v.fields[0].field, "inbound_port"),
            other => panic!("应为字段级校验错误，实得 {other:?}"),
        }
        // 未被改动
        assert_eq!(plugin.config.read().await.inbound_port, 9231);
    }

    #[tokio::test]
    async fn status_reports_server_not_running_when_inbound_disabled() {
        let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default(), tdir()));
        let resp = call(plugin, "status", None).await.unwrap();
        let v = resp.serialize().unwrap();
        assert_eq!(v["inbound_running"], false);
        assert_eq!(v["inbound_enabled"], false);
    }

    #[tokio::test]
    async fn unknown_path_is_not_found() {
        let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default(), tdir()));
        assert!(call(plugin.clone(), "bogus", None).await.is_err());
        // 配置文档之外无其它节点
        assert!(plugin.stat(&vctx(), "bogus").await.is_err());
    }
}
