//! 探针插件的门面与债表：端口与债行都由本 crate 自己申报。
//!
//! 债是**惰性**的：只有会话 id 以 `probe-` 开头时才欠账。这让插件能进默认装配
//! （默认树每会话都会问一遍各债务源），却不改变任何既有会话的驱动步数——验收用它
//! 自己的会话前缀把债叫出来。

use std::sync::Arc;

use async_trait::async_trait;
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PortProvider, CAPABILITY_VISITOR, PATH,
};

use crate::port::{ProbePort, PROBE_PORT};

/// 插件身份 = 目录名（E-001）。身份与端口名分开：一个插件可以申报多个端口。
const PROBE_PLUGIN: &str = "probe";

/// 只有这个前缀的会话才欠一笔探针债（验收夹具用 `probe-acceptance` 叫出它）。
const PROBE_SESSION_PREFIX: &str = "probe-";

/// 探针的债表：一笔 `probe_echo` 端口的债，身份由会话身份派生。
struct ProbeDebts;

impl DebtSource for ProbeDebts {
    fn rows(&self, request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        if !request.session_id.starts_with(PROBE_SESSION_PREFIX) {
            return Vec::new();
        }
        let id = format!("probe:{}", request.session_id);
        if facts.closing(&id).is_some() {
            return Vec::new();
        }
        vec![DebtSpec::new(
            "probe-row",
            PROBE_PORT,
            id,
            serde_json::json!({ "text": "hello" }),
        )]
    }
}

pub(crate) struct ProbePlugin;

impl ProbePlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。
    pub(crate) fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ProbePlugin) as Arc<dyn Plugin>
    }

    fn metadata() -> PluginMeta {
        PluginMeta::new(PROBE_PLUGIN, "Probe")
            .with_description("验收探针：申报 probe_echo 端口与一笔按会话前缀惰性的债")
            .with_version("0.1.0")
            .with_hidden(true)
    }
}

#[async_trait]
impl Plugin for ProbePlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[probe] 未知子命令: {path}（执行走端口提供者，不经 route）"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            let provider: Arc<dyn PortProvider> = Arc::new(ProbePort);
            symbio_plugin_api::register_ports(&ctx, provider.as_ref()).await;
            visitor
                .put(debt_row("probe-debts", Arc::new(ProbeDebts)))
                .await;
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PROBE_PLUGIN, ProbePlugin::build, dyn Plugin);
