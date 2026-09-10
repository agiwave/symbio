//! 能力管理器的默认实现（跨插件共享设施）
//!
//! ## 为什么放在 `symbio_core`
//!
//! 插件之间**互相不可见**（`plugins/mod.rs` 的架构约束），只能依赖 `symbio_core`。
//! 会话编排（session 插件）需要自行构造 `CapabilityManager` 来收集各插件贡献的工具，
//! 因此 `DefaultToolManager` 必须上浮为共享设施，而不能停留在某个插件的私有模块里。
//!
//! 迁移前位置：`plugins/agent/core/default_tool_manager.rs`

use crate::symbio_core::{
    Capability, CapabilityManager, CapabilityMeta, InvokeRequest, InvokeResponse,
    ModelProviderEntry, PluginError, PluginPayload,
};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 默认能力管理器：内存 HashMap 实现，一次会话请求一个实例
///
/// 除工具外，同时承载 Phase B 扩充的两组注册（与工具同一 traverse 收集机制）：
/// - `providers`：模型服务目录（AI 对话能力）
/// - `system_prompts`：系统提示词（按名称保序）
pub struct DefaultToolManager {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Capability>>>>,
    providers: Arc<RwLock<IndexMap<String, ModelProviderEntry>>>,
    system_prompts: Arc<RwLock<IndexMap<String, String>>>,
}

impl DefaultToolManager {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            providers: Arc::new(RwLock::new(IndexMap::new())),
            system_prompts: Arc::new(RwLock::new(IndexMap::new())),
        }
    }
}

impl Default for DefaultToolManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityManager for DefaultToolManager {
    async fn register(&self, tool: Arc<dyn Capability>) {
        let name = tool.name();
        let mut tools = self.tools.write().await;
        tools.insert(name, tool);
    }

    async fn list_capability(&self) -> Vec<CapabilityMeta> {
        let tools = self.tools.read().await;
        tools.values().map(|t| t.meta()).collect()
    }

    async fn invoke(
        &self,
        name: &str,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let tool = {
            let tools = self.tools.read().await;
            tools.get(name).cloned()
        };

        match tool {
            Some(tool) => tool.execute(ctx).await,
            None => Err(PluginError::NotFound(format!("Tool not found: {name}"))),
        }
    }

    async fn has_capability(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        tools.contains_key(name)
    }

    async fn register_model_provider(&self, entry: ModelProviderEntry) {
        let mut providers = self.providers.write().await;
        providers.insert(entry.provider_id.clone(), entry);
    }

    async fn list_model_providers(&self) -> Vec<ModelProviderEntry> {
        let providers = self.providers.read().await;
        providers.values().cloned().collect()
    }

    async fn get_model_provider(&self, provider_id: &str) -> Option<ModelProviderEntry> {
        let providers = self.providers.read().await;
        providers.get(provider_id).cloned()
    }

    async fn register_system_prompt(&self, name: &str, prompt: String) {
        let mut prompts = self.system_prompts.write().await;
        prompts.insert(name.to_string(), prompt);
    }

    async fn list_system_prompts(&self) -> Vec<(String, String)> {
        let prompts = self.system_prompts.read().await;
        prompts
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::model::model_config::ModelConfig;
    use crate::symbio_core::schemas::session::chat_message::ChatMessage;
    use crate::symbio_core::{ModelProvider, ProtocolEvent};
    use async_trait::async_trait;
    use serde_json::Value;

    /// 最小协议桩：仅用于验证注册存储语义，不发起真实请求
    struct MockProvider {
        tag: String,
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        fn get_api_url(&self, _config: &ModelConfig) -> String {
            self.tag.clone()
        }

        fn get_headers(&self, _config: &ModelConfig) -> reqwest::header::HeaderMap {
            reqwest::header::HeaderMap::new()
        }

        fn prepare_request(
            &self,
            _config: &ModelConfig,
            _system_prompt: &str,
            _messages: &[ChatMessage],
            _tools: &[CapabilityMeta],
        ) -> Value {
            serde_json::json!({})
        }

        fn parse_response_line(&self, _line: &str) -> Vec<ProtocolEvent> {
            Vec::new()
        }
    }

    fn provider_entry(id: &str, description: &str) -> ModelProviderEntry {
        ModelProviderEntry {
            provider_id: id.to_string(),
            protocol_id: "openai_chat".to_string(),
            description: description.to_string(),
            system_prompt: None,
            config: ModelConfig::default(),
            rate_limit_ms: 0,
            is_default: false,
            provider: Arc::new(MockProvider {
                tag: id.to_string(),
            }),
        }
    }

    #[tokio::test]
    async fn provider_roundtrip_preserves_registration_order() {
        let mgr = DefaultToolManager::new();
        mgr.register_model_provider(provider_entry("p1", "第一个"))
            .await;
        mgr.register_model_provider(provider_entry("p2", "第二个"))
            .await;
        mgr.register_model_provider(provider_entry("p3", "第三个"))
            .await;

        let listed = mgr.list_model_providers().await;
        let ids: Vec<&str> = listed.iter().map(|e| e.provider_id.as_str()).collect();
        assert_eq!(ids, vec!["p1", "p2", "p3"]);

        assert!(mgr.get_model_provider("p2").await.is_some());
        assert!(mgr.get_model_provider("missing").await.is_none());
    }

    #[tokio::test]
    async fn provider_overwrite_keeps_single_entry() {
        let mgr = DefaultToolManager::new();
        mgr.register_model_provider(provider_entry("p1", "first"))
            .await;
        mgr.register_model_provider(provider_entry("p1", "second"))
            .await;

        let listed = mgr.list_model_providers().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].description, "second");
    }

    #[tokio::test]
    async fn system_prompt_overwrite_keeps_first_registration_slot() {
        let mgr = DefaultToolManager::new();
        mgr.register_system_prompt("default", "默认提示词".to_string())
            .await;
        mgr.register_system_prompt("p1", "P1 提示词".to_string())
            .await;
        mgr.register_system_prompt("default", "默认提示词（覆盖）".to_string())
            .await;
        mgr.register_system_prompt("p2", "P2 提示词".to_string())
            .await;

        let prompts = mgr.list_system_prompts().await;
        assert_eq!(
            prompts,
            vec![
                ("default".to_string(), "默认提示词（覆盖）".to_string()),
                ("p1".to_string(), "P1 提示词".to_string()),
                ("p2".to_string(), "P2 提示词".to_string()),
            ]
        );
    }
}
