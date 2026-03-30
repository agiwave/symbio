//! Home 插件工厂 - 创建根插件，并组装子插件

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::HomePlugin;
use crate::plugins::work::WorkFactory;
use crate::plugins::agent::AgentFactory;
use crate::plugins::setting::SettingFactory;
use std::sync::Arc;
use serde_json::Value;

pub struct HomeFactory;

impl HomeFactory {
    pub fn new() -> Self {
        HomeFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for HomeFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "home".to_string(),
            description: "Symbio 主插件".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        // 使用各子插件的工厂创建实例
        let work = WorkFactory::new().create(None, None);
        let agent = AgentFactory::new().create(None, None);
        let setting = SettingFactory::new().create(None, None);

        Arc::new(HomePlugin::new(work, agent, setting))
    }
}
