//! Setting 插件工厂

use crate::symbio_core::traits::{Plugin, PluginFactory};
use crate::symbio_core::types::PluginMeta;
use super::SettingPlugin;
use std::sync::{Arc, Weak};
use serde_json::Value;

pub struct SettingFactory;

impl SettingFactory {
    pub fn new() -> Self {
        SettingFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for SettingFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "setting".to_string(),
            description: "设置管理插件".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Weak<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(SettingPlugin::new(parent))
    }
}
