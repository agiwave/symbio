//! Formatter Factory

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::FormatterPlugin;
use std::sync::Arc;
use serde_json::Value;

#[derive(Clone)]
pub struct FormatterFactory;

impl FormatterFactory {
    pub fn new() -> Self {
        FormatterFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for FormatterFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "formatter".to_string(),
            description: "格式化插件工厂".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }
    
    fn create(&self, _parent: Option<&dyn Plugin>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(FormatterPlugin::new())
    }
}

impl Default for FormatterFactory {
    fn default() -> Self {
        Self::new()
    }
}
