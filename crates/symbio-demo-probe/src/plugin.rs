//! 假插件的门面与债表：端口与债行都由本 crate 自己申报。

use std::sync::Arc;

use async_trait::async_trait;
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PortProvider, CAPABILITY_VISITOR, PATH,
};

use crate::port::{DemoPort, DEMO_PORT};

/// 插件身份（与端口名分开：一个插件可以申报多个端口，身份不复用端口名）。
const DEMO_PLUGIN: &str = "demo-plugin";

/// 假插件的债表：一笔 `demo` 端口的债，身份由会话身份派生。
struct DemoDebts;

impl DebtSource for DemoDebts {
    fn rows(&self, request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let id = format!("demo:{}", request.session_id);
        if facts.closing(&id).is_some() {
            return Vec::new();
        }
        vec![DebtSpec::new(
            "demo-row",
            DEMO_PORT,
            id,
            serde_json::json!({ "text": "hello" }),
        )]
    }
}

pub struct DemoPlugin;

impl DemoPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(DemoPlugin) as Arc<dyn Plugin>
    }

    fn metadata() -> PluginMeta {
        PluginMeta::new(DEMO_PLUGIN, "Demo")
            .with_description("验收假插件：申报 demo 端口与一笔债")
            .with_version("0.1.0")
            .with_hidden(true)
    }
}

#[async_trait]
impl Plugin for DemoPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[demo] 未知子命令: {path}（执行走端口提供者，不经 route）"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            let provider: Arc<dyn PortProvider> = Arc::new(DemoPort);
            symbio_plugin_api::register_ports(&ctx, provider.as_ref()).await;
            visitor
                .put(debt_row("demo-debts", Arc::new(DemoDebts)))
                .await;
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(DEMO_PLUGIN, DemoPlugin::build, dyn Plugin);

#[cfg(test)]
mod tests;
