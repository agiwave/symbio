//! 能力管理器的默认实现（跨插件共享设施）
//!
//! ## 为什么放在 `symbio_core`
//!
//! 插件之间**互相不可见**（`plugins/mod.rs` 的架构约束），只能依赖 `symbio_core`。
//! 会话编排（session 插件）需要自行构造 `CapabilityVisitor` 来收集各插件贡献的工具，
//! 因此 `DefaultToolVisitor` 必须作为共享设施定义在 core，不得放进任何插件的私有模块。

use crate::symbio_core::{
    invoke_capability, Capability, CapabilityMeta, CapabilityVisitor, InvokeRequest,
    InvokeResponse, ModelProvider, PluginError, PluginPayload, VdfsProvider,
};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 默认能力管理器：内存 HashMap 实现，一次会话请求一个实例
///
/// 除工具外，同时承载四组注册（与工具同一 traverse 收集机制）：
/// - `provider`：当前生效的模型服务（单槽；model 插件按上下文解析出
///   唯一生效 Provider 后注册，重复注册覆盖）
/// - `system_prompts`：系统提示词（按名称保序；**全部**送达模型，不做竞争）
/// - `vdfs_providers`：VDFS 挂载点（按挂载名去重，`order` 升序对外）。
///   这是 **LLM 可控挂载机制**的清单（子智能体经 `SubAgentVisitor` 在此加
///   `agent/<id>/` 作用域前缀，预留按作用域 / 白名单裁剪 LLM 可见资源）；
///   前端 / 系统链路**不经过它**（走 `Plugin::get_vfs_provider`）。当前
///   `vdfs_*` 工具取根走 `vdfs_root` 单槽，本清单暂无消费方——**预留，勿删**。
/// - `vdfs_root`：VDFS 根 provider（单槽；组合容器注册，访问层据此转发）
pub struct DefaultToolVisitor {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Capability>>>>,
    provider: Arc<RwLock<Option<Arc<dyn ModelProvider>>>>,
    system_prompts: Arc<RwLock<IndexMap<String, String>>>,
    vdfs_providers: Arc<RwLock<IndexMap<String, Arc<dyn VdfsProvider>>>>,
    vdfs_root: Arc<RwLock<Option<Arc<dyn VdfsProvider>>>>,
}

impl DefaultToolVisitor {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            provider: Arc::new(RwLock::new(None)),
            system_prompts: Arc::new(RwLock::new(IndexMap::new())),
            vdfs_providers: Arc::new(RwLock::new(IndexMap::new())),
            vdfs_root: Arc::new(RwLock::new(None)),
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
            Some(tool) => invoke_capability(tool.as_ref(), ctx).await,
            None => Err(PluginError::NotFound(format!("Tool not found: {name}"))),
        }
    }

    async fn has_capability(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        tools.contains_key(name)
    }

    async fn register_model_provider(&self, provider: Arc<dyn ModelProvider>) {
        let mut slot = self.provider.write().await;
        *slot = Some(provider);
    }

    async fn get_model_provider(&self) -> Option<Arc<dyn ModelProvider>> {
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

    async fn register_vdfs_provider(&self, name: &str, provider: Arc<dyn VdfsProvider>) {
        let mut slot = self.vdfs_providers.write().await;
        // 同目录名覆盖（保留先注册槽位，IndexMap 语义与工具注册一致）
        slot.insert(name.to_string(), provider);
    }

    async fn list_vdfs_providers(&self) -> Vec<(String, Arc<dyn VdfsProvider>)> {
        let slot = self.vdfs_providers.read().await;
        let out: Vec<(String, Arc<dyn VdfsProvider>)> = slot
            .iter()
            .map(|(name, p)| (name.clone(), p.clone()))
            .collect();
        // 注册顺序即稳定顺序（导航排序归容器：按 `PluginMeta::order` 在合成目录时排）
        out
    }

    async fn get_vdfs_provider(&self, name: &str) -> Option<Arc<dyn VdfsProvider>> {
        let slot = self.vdfs_providers.read().await;
        slot.get(name).cloned()
    }

    async fn register_vdfs_root(&self, provider: Arc<dyn VdfsProvider>) {
        let mut slot = self.vdfs_root.write().await;
        *slot = Some(provider);
    }

    async fn get_vdfs_root(&self) -> Option<Arc<dyn VdfsProvider>> {
        let slot = self.vdfs_root.read().await;
        slot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::session::chat_message::ChatMessage;
    use crate::symbio_core::turn::TurnOutput;
    use crate::symbio_core::{CapabilityMeta, ExecEnv, PluginError};
    use async_trait::async_trait;

    /// 最小模型服务桩：实现纯 trait 契约，仅用于验证注册存储语义，不发起真实请求
    struct MockProvider {
        tag: String,
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        fn provider_id(&self) -> &str {
            &self.tag
        }

        fn api_protocol(&self) -> &str {
            "openai_chat"
        }

        fn rate_limit_ms(&self) -> u64 {
            0
        }

        fn max_context_tokens(&self) -> u32 {
            262_144
        }

        async fn effective_context_tokens(&self) -> u32 {
            262_144
        }

        async fn execute_turn(
            &self,
            _system_prompt: &str,
            _messages: &[ChatMessage],
            _tools: &[CapabilityMeta],
            _root_id: &str,
            _env: &ExecEnv,
        ) -> Result<TurnOutput, PluginError> {
            Err(PluginError::InternalError("mock".to_string()))
        }
    }

    fn provider(id: &str) -> Arc<dyn ModelProvider> {
        Arc::new(MockProvider {
            tag: id.to_string(),
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
        assert_eq!(got.provider_id(), "p1");
    }

    #[tokio::test]
    async fn provider_overwrite_replaces_single_slot() {
        let mgr = DefaultToolVisitor::new();
        mgr.register_model_provider(provider("p1")).await;
        mgr.register_model_provider(provider("p2")).await;

        // 单槽覆盖：后注册者生效
        let got = mgr.get_model_provider().await;
        let got = got.expect("覆盖注册后仍应可取回");
        assert_eq!(got.provider_id(), "p2");
    }

    /// 同名覆盖只改内容、**不改槽位**（顺序 = 首次注册顺序）
    #[tokio::test]
    async fn system_prompt_overwrite_keeps_first_registration_slot() {
        let mgr = DefaultToolVisitor::new();
        mgr.register_system_prompt("persona", "默认提示词".to_string())
            .await;
        mgr.register_system_prompt("work", "记忆".to_string()).await;
        mgr.register_system_prompt("persona", "默认提示词（覆盖）".to_string())
            .await;
        mgr.register_system_prompt("agent", "人格".to_string())
            .await;

        assert_eq!(
            mgr.list_system_prompts().await,
            vec![
                ("persona".to_string(), "默认提示词（覆盖）".to_string()),
                ("work".to_string(), "记忆".to_string()),
                ("agent".to_string(), "人格".to_string()),
            ]
        );
    }
}
