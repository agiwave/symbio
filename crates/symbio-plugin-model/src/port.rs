//! model 端口的世界提供者（D-298 M1）：解析与适配器构造收在 model 插件一侧。
//!
//! 环（loop）只按端口名取——provider 怎么解析（严格查 `preferred` → 落回当次
//! 生效者）、适配器怎么构造，都是这里的私事。`ModelPort` 在 traverse 里随能力
//! 目录一起装：生效者缺席 ⇒ 不注册端口行 ⇒ 环在调用点响亮失败（落失败事实
//! 并中止），而不是在这里猜一个别的模型。

use std::sync::Arc;

use symbio_plugin_api::port::PortProvider;
use symbio_plugin_api::ports;
use symbio_plugin_api::{
    CapabilityRow, ExecAbortSignal, LlmAdapter, ModelProvider, ProviderLlmAdapter,
};

/// `port.model` 的提供者：当次生效者 + 完整目录（严格解析用）。
///
/// 目录 = 全部已启用的 provider（与能力目录同一批 `BoundProvider`）；生效者
/// 必然是目录一员（不在 ⇒ traverse 不注册本行，见模块文档）。`preferred`
/// 查目录不中生 ⇒ 落回生效者——降级方向只有这一处定义。
pub(crate) struct ModelPort {
    active: Arc<dyn ModelProvider>,
    catalog: Vec<Arc<dyn ModelProvider>>,
}

impl ModelPort {
    pub(crate) fn new(
        active: Arc<dyn ModelProvider>,
        catalog: Vec<Arc<dyn ModelProvider>>,
    ) -> Self {
        Self { active, catalog }
    }

    /// 注册行（本提供者只服务一个端口，行数恒为 1）。
    pub(crate) fn rows(self: Arc<Self>) -> Vec<CapabilityRow> {
        symbio_plugin_api::port::port_rows(&(self as Arc<dyn PortProvider>))
    }
}

impl PortProvider for ModelPort {
    fn ports(&self) -> &'static [&'static str] {
        &[ports::MODEL]
    }

    fn provider_id(&self) -> &str {
        self.active.provider_id()
    }

    fn llm_round_trip(
        &self,
        preferred: Option<&str>,
        system_prompt: &str,
        abort: ExecAbortSignal,
    ) -> Option<Arc<dyn LlmAdapter>> {
        let provider = preferred
            .and_then(|id| self.catalog.iter().find(|p| p.provider_id() == id))
            .unwrap_or(&self.active);
        Some(Arc::new(ProviderLlmAdapter::for_turn(
            provider.clone(),
            system_prompt,
            abort,
        )))
    }

    fn model(&self) -> Option<Arc<dyn ModelProvider>> {
        Some(self.active.clone())
    }
}
