use crate::symbio_core::{
    Capability, CapabilityManager, CapabilityMeta, InvokeRequest, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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
