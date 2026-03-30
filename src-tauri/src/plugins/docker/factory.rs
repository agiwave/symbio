//! Docker 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::DockerPlugin;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct DockerFactory;

impl DockerFactory {
    pub fn new() -> Self {
        DockerFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for DockerFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "docker".to_string(),
            description: "Docker 执行环境插件工厂".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(DockerPlugin::new())
    }
}

impl Default for DockerFactory {
    fn default() -> Self {
        Self::new()
    }
}
