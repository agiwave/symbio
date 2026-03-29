//! Calculator Factory

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::CalculatorPlugin;
use std::sync::Arc;
use serde_json::Value;

#[derive(Clone)]
pub struct CalculatorFactory;

impl CalculatorFactory {
    pub fn new() -> Self {
        CalculatorFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for CalculatorFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "calculator".to_string(),
            description: "计算器插件工厂".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }
    
    fn create(&self, _parent: Option<&dyn Plugin>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(CalculatorPlugin::new())
    }
}

impl Default for CalculatorFactory {
    fn default() -> Self {
        Self::new()
    }
}
