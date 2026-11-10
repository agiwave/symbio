//! `probe_echo` 端口的执行面：把载荷里的字符串原样回显。

use std::sync::Arc;

use async_trait::async_trait;
use symbio_plugin_api::{PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider};

/// 端口名（唯一来源：本插件）。选 `probe_echo` 而不是 `probe`，避免与验证夹具里的
/// 端口名混淆——两个名字都合法，但同名会让日志里分不清「哪个探针」。
pub(crate) const PROBE_PORT: &str = "probe_echo";

pub(crate) struct ProbePort;

impl PortProvider for ProbePort {
    fn ports(&self) -> Vec<String> {
        vec![PROBE_PORT.to_string()]
    }

    fn provider_id(&self) -> &str {
        PROBE_PORT
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(ProbeExecutor))
    }
}

struct ProbeExecutor;

#[async_trait]
impl PortExecutor for ProbeExecutor {
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
