//! 能力管理器的默认实现（跨插件共享设施）
//!
//! ## 为什么放在 `symbio_core`
//!
//! 插件之间**互相不可见**（`plugins/mod.rs` 的架构约束），只能依赖 `symbio_core`。
//! 会话编排（session 插件）需要自行构造 `CapabilityVisitor` 来收集各插件贡献的工具，
//! 因此 `DefaultToolVisitor` 必须上浮为共享设施，而不能停留在某个插件的私有模块里。
//!
//! 迁移前位置：`plugins/agent/core/default_tool_visitor.rs`

use crate::symbio_core::{
    Capability, CapabilityVisitor, CapabilityMeta, InvokeRequest, InvokeResponse, ModelProvider,
    PluginError, PluginPayload,
};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 默认能力管理器：内存 HashMap 实现，一次会话请求一个实例
///
/// 除工具外，同时承载两组注册（与工具同一 traverse 收集机制）：
/// - `provider`：当前生效的模型服务（单槽；model 插件按上下文解析出
///   唯一生效 Provider 后注册，重复注册覆盖）
/// - `system_prompts`：系统提示词（按名称保序）
pub struct DefaultToolVisitor {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Capability>>>>,
    provider: Arc<RwLock<Option<Arc<ModelProvider>>>>,
    system_prompts: Arc<RwLock<IndexMap<String, String>>>,
}

impl DefaultToolVisitor {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            provider: Arc::new(RwLock::new(None)),
            system_prompts: Arc::new(RwLock::new(IndexMap::new())),
        }
    }
}

impl Default for DefaultToolVisitor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityVisitor for DefaultToolVisitor {
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

    async fn register_model_provider(&self, provider: Arc<ModelProvider>) {
        let mut slot = self.provider.write().await;
        *slot = Some(provider);
    }

    async fn get_model_provider(&self) -> Option<Arc<ModelProvider>> {
        let slot = self.provider.read().await;
        slot.clone()
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
    use crate::symbio_core::schemas::session::chat_message::ChatMessage;
    use crate::symbio_core::{CapabilityMeta, ModelProtocol, ProtocolEvent};
    use async_trait::async_trait;
    use serde_json::Value;

    /// 最小协议桩：仅用于验证注册存储语义，不发起真实请求
    struct MockProtocol {
        tag: String,
    }

    #[async_trait]
    impl ModelProtocol for MockProtocol {
        fn get_api_url(&self, _provider: &ModelProvider) -> String {
            self.tag.clone()
        }

        fn get_headers(&self, _provider: &ModelProvider) -> reqwest::header::HeaderMap {
            reqwest::header::HeaderMap::new()
        }

        fn prepare_request(
            &self,
            _provider: &ModelProvider,
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

    fn provider(id: &str) -> Arc<ModelProvider> {
        Arc::new(ModelProvider {
            provider_id: id.to_string(),
            protocol_id: "openai_chat".to_string(),
            system_prompt: None,
            rate_limit_ms: 0,
            provider: id.to_string(),
            api_base: String::new(),
            api_key: None,
            model: String::new(),
            temperature: 0.7,
            max_tokens: None,
            max_context_tokens: 262_144,
            reserved_tokens: 4_096,
            timeout_secs: 300,
            api_protocol: "openai_chat".to_string(),
            store: false,
            reasoning: None,
            protocol: Arc::new(MockProtocol {
                tag: id.to_string(),
            }),
        })
    }

    #[tokio::test]
    async fn provider_slot_set_get_and_missing() {
        let mgr = DefaultToolVisitor::new();
        // 未注册 → None
        assert!(mgr.get_model_provider().await.is_none());

        // 注册后可取回，身份字段一致
        mgr.register_model_provider(provider("p1")).await;
        let got = mgr.get_model_provider().await;
        let got = got.expect("注册后应可取回");
        assert_eq!(got.provider_id, "p1");
        assert_eq!(got.protocol.get_api_url(&got), "p1");
    }

    #[tokio::test]
    async fn provider_overwrite_replaces_single_slot() {
        let mgr = DefaultToolVisitor::new();
        mgr.register_model_provider(provider("p1")).await;
        mgr.register_model_provider(provider("p2")).await;

        // 单槽覆盖：后注册者生效
        let got = mgr.get_model_provider().await;
        let got = got.expect("覆盖注册后仍应可取回");
        assert_eq!(got.provider_id, "p2");
    }

    #[tokio::test]
    async fn system_prompt_overwrite_keeps_first_registration_slot() {
        let mgr = DefaultToolVisitor::new();
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
