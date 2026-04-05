//! Work 插件工厂

use crate::symbio_core::traits::{Plugin, PluginFactory};
use crate::symbio_core::types::PluginMeta;
use super::WorkPlugin;
use std::sync::{Arc, Weak};
use serde_json::Value;

pub struct WorkFactory;

impl WorkFactory {
    pub fn new() -> Self {
        WorkFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for WorkFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "work".to_string(),
            description: "工作区路径管理插件".to_string(),
            version: "0.2.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Weak<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        let parent_weak = parent;
        Arc::new(WorkPlugin::new(parent_weak))
    }
}
