//! `demo` 端口的执行面：把载荷里的字符串原样回显。

use std::sync::Arc;

use async_trait::async_trait;
use symbio_plugin_api::{PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider};

/// 端口名（唯一来源：本插件）。
pub(crate) const DEMO_PORT: &str = "demo";

pub(crate) struct DemoPort;

impl PortProvider for DemoPort {
    fn ports(&self) -> &'static [&'static str] {
        &[DEMO_PORT]
    }

    fn provider_id(&self) -> &str {
        DEMO_PORT
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(DemoExecutor))
    }
}

struct DemoExecutor;

#[async_trait]
impl PortExecutor for DemoExecutor {
    async fn execute(&self, _ctx: PortExecContext, input: PortInput) -> PortOutcome {
        let text = input
            .payload
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        PortOutcome {
            facts: vec![serde_json::json!({ "echo": text })],
            ..PortOutcome::ok()
        }
    }
}
