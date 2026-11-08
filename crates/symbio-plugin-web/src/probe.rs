//! 探针端口（**仅测试编译**，见 `lib.rs` 的 `#[cfg(test)] mod probe;`）。
//!
//! 它钉的是横向扩展的验收句：**新增一个世界端口，改动架构文件数 = 0**（D-394 /
//! PTS-007）。这里申报的端口 `probe` 在仓内没有任何生产消费方；它存在的全部意义是
//! 证明「端口声明 + 一笔债务 + 执行体」三件套可以只落在一个**能力插件** crate 内——
//! 不碰 `kernel` / `sdk` / `plugin-api` / `loop` / `session`。
//!
//! 出厂编译里没有本文件：`probe` 不进装配、不进能力账本，`--full` 的 EX-001 /
//! EX-006 读数因此不受影响。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use symbio_plugin_api::debt::DebtSpec;
use symbio_plugin_api::port::{
    PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider,
};

/// 探针端口名：合法（无 `:` 等保留字符），且与仓内任何世界端口都不重名。
pub(crate) const PROBE_PORT: &str = "probe";
/// 探针提供者 id（端口行名后缀）。
pub(crate) const PROBE_ID: &str = "probe";

/// 最小提供者：申报一个端口、交出一份执行面。
pub(crate) struct ProbePort;

impl PortProvider for ProbePort {
    fn ports(&self) -> &'static [&'static str] {
        &["probe"]
    }

    fn provider_id(&self) -> &str {
        PROBE_ID
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(ProbeExecutor))
    }
}

/// 探针的一笔债：端口名、行名、幂等键都来自本 crate，机制侧不认识 `probe`。
pub(crate) fn debt() -> DebtSpec {
    DebtSpec::new(
        "probe-step",
        PROBE_PORT,
        "probe:step-1",
        json!({ "step": 1 }),
    )
}

/// 最小执行体：把载荷原样交回一条事实——证明「执行体就位」不需要机制侧认识本端口。
pub(crate) struct ProbeExecutor;

#[async_trait]
impl PortExecutor for ProbeExecutor {
    async fn execute(&self, _ctx: PortExecContext, input: PortInput) -> PortOutcome {
        let mut outcome = PortOutcome::ok();
        outcome.facts.push(input.payload);
        outcome
    }
}

#[cfg(test)]
#[path = "probe.test.rs"]
mod tests;
