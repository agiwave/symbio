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
    Capability, CapabilityManager, CapabilityMeta, InvokeRequest, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 默认能力管理器：内存 HashMap 实现，一次会话请求一个实例
pub struct DefaultToolManager {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Capability>>>>,
}

impl DefaultToolManager {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
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
}
