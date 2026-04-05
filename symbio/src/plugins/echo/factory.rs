//! Echo Factory

use crate::symbio_core::traits::{Plugin, PluginFactory};
use crate::symbio_core::types::PluginMeta;
use super::plugin::EchoPlugin;
use serde_json::Value;
use std::sync::{Arc, Weak};

#[derive(Clone)]
pub struct EchoFactory;

impl EchoFactory {
    pub fn new() -> Self {
        EchoFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for EchoFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "echo".to_string(),
            description: "回显插件工厂".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }
    
    fn create(&self, _parent: Option<Weak<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(EchoPlugin::new())
    }
}

impl Default for EchoFactory {
    fn default() -> Self {
        Self::new()
    }
}
