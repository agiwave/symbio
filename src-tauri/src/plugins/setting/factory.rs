//! Setting 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::SettingPlugin;
use std::sync::Arc;
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

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(SettingPlugin::new())
    }
}
