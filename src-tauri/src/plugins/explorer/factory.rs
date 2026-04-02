//! Explorer 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::ExplorerPlugin;
use std::sync::{Arc, Weak};
use serde_json::Value;

pub struct ExplorerFactory;

impl ExplorerFactory {
    pub fn new() -> Self {
        ExplorerFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for ExplorerFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "explorer".to_string(),
            description: "工作区资源浏览器".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Weak<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        let parent_weak = parent;
        Arc::new(ExplorerPlugin::new(parent_weak))
    }
}
