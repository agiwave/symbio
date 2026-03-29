//! Agent Factory

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::Agent;
use serde_json::Value;
use std::sync::Arc;

/// Agent 工厂
/// 
/// 用于创建 Agent 实例，支持分形嵌套结构
pub struct AgentFactory {
    meta: PluginMeta,
}

impl AgentFactory {
    pub fn new() -> Self {
        AgentFactory {
            meta: PluginMeta {
                name: "agent".to_string(),
                description: "通用的插件容器，可以管理子插件实例，支持嵌套".to_string(),
                version: "0.1.0".to_string(),
                input: None,
                output: None,
                author: Some("Symbio Team".to_string()),
            },
        }
    }
}

impl Default for AgentFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PluginFactory for AgentFactory {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }

    fn create(&self, _parent: Option<&dyn Plugin>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        // 子 Agent 使用全局注册表，支持分形嵌套
        Arc::new(Agent::new())
    }
}
