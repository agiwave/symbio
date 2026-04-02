//! Doc 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::DocPlugin;
use std::sync::{Arc, Weak};
use serde_json::Value;

pub struct DocFactory;

impl DocFactory {
    pub fn new() -> Self {
        DocFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for DocFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "doc".to_string(),
            description: "文档管理插件".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Weak<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(DocPlugin::new(parent))
    }
}
