//! 网关插件 —— 负责「本应用如何被调用」以及「本应用调用谁」
//!
//! 挂载于 worker composite 之下（与 `model`/`session` 平级），故 `ctx.parent()` 即为
//! worker，调用 `parent.route(ctx)` 等价于既有的 `root.route`：能转发前端会发起的全部路径
//!（`session/*`、`model/*`、`entities/*` …）。因此本插件**无需任何新全局注册表**。

use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta, PluginPayload,
    CONFIG_GET, CONFIG_SET, PATH, PLUGIN_GATEWAY,
};
use async_trait::async_trait;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::config::GatewayConfig;
use super::server;

pub struct GatewayPlugin {
    config: Arc<RwLock<GatewayConfig>>,
    /// 父插件（worker composite）弱引用，用于转发请求与上行落盘
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
    /// 入站服务句柄（为空表示未启动）
    server: Arc<RwLock<Option<server::ServerHandle>>>,
}

impl GatewayPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例（submit_object_creator! 自注册）
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let config: GatewayConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let parent = ctx.parent();
        let arc = Arc::new(Self::new(parent, config));
        // 启动期若已启用入站服务，则拉起监听（配置变更后由 config/set 重建）
        let arc2 = arc.clone();
        tokio::spawn(async move {
            arc2.start_server().await;
        });
        arc
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: GatewayConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
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
            CONFIG_GET => {
                let cfg = self.config.read().await;
                Ok(PluginPayload::new(&*cfg))
            }
            CONFIG_SET => {
                let next: GatewayConfig = ctx.payload()?;
                {
                    let mut cfg = self.config.write().await;
                    *cfg = next;
                }
                // 与 SettingPlugin 同款：交父级上行落盘（save_config → home 聚合写盘）
                if let Some(p) = self.get_parent().await {
                    let save_ctx = ctx.fork();
                    save_ctx.set(PATH, "save_config".to_string());
                    p.route(save_ctx).await?;
                }
                // 端口/开关/协议变更需重建监听
                self.apply_config().await;
                Ok(PluginPayload::new(&serde_json::json!({ "ok": true })))
            }
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
                    "outbound_protocol": cfg.outbound_protocol,
                    "outbound_endpoint": cfg.outbound_endpoint,
                })))
            }
            _ => Err(PluginError::NotFound(format!(
                "[gateway] 未知路径: {path}"
            ))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_GATEWAY, GatewayPlugin::build, dyn Plugin);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::{
        InvokeRequestExt, InvokeResponse, PATH, Plugin, PluginPayload, SimpleRequest,
    };
    use std::sync::Arc;

    /// 以子插件直接收的**已剥离前缀**路径（如 `config/get`）调用 route，
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

    #[tokio::test]
    async fn config_get_returns_current_config() {
        let cfg = GatewayConfig {
            outbound_protocol: "http".into(),
            outbound_endpoint: "http://remote:9231".into(),
            ..GatewayConfig::default()
        };
        let plugin = Arc::new(GatewayPlugin::new(None, cfg));
        let resp = call(plugin, "config/get", None).await.unwrap();
        let got: GatewayConfig = resp.get().unwrap();
        assert_eq!(got.outbound_protocol, "http");
        assert_eq!(got.outbound_endpoint, "http://remote:9231");
        assert_eq!(got.inbound_port, 9231);
    }

    #[tokio::test]
    async fn config_set_persists_and_is_readable_back() {
        let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default()));

        let mut next = GatewayConfig::default();
        next.outbound_protocol = "http".into();
        next.outbound_endpoint = "http://other:9231".into();
        let set_resp = call(
            plugin.clone(),
            "config/set",
            Some(serde_json::to_value(&next).unwrap()),
        )
        .await
        .unwrap();
        // config/set 返回 { ok: true }
        assert_eq!(set_resp.serialize().unwrap()["ok"], true);

        // 随后 config/get 应读回新值（父级为 None，不触发落盘/启服，仅内存生效）
        let got = call(plugin.clone(), "config/get", None).await.unwrap();
        let reread: GatewayConfig = got.get().unwrap();
        assert_eq!(reread.outbound_protocol, "http");
        assert_eq!(reread.outbound_endpoint, "http://other:9231");
    }

    #[tokio::test]
    async fn status_reports_server_not_running_when_inbound_disabled() {
        let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default()));
        let resp = call(plugin, "status", None).await.unwrap();
        let v = resp.serialize().unwrap();
        assert_eq!(v["inbound_running"], false);
        assert_eq!(v["inbound_enabled"], false);
    }

    #[tokio::test]
    async fn unknown_path_is_not_found() {
        let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default()));
        let r = call(plugin, "config/delete", None).await;
        assert!(r.is_err());
    }
}

