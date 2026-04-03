//! Note 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::NotePlugin;
use std::sync::{Arc, Weak};
use serde_json::Value;

pub struct NoteFactory;

impl NoteFactory {
    pub fn new() -> Self {
        NoteFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for NoteFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "note".to_string(),
            description: "笔记管理插件".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Weak<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(NotePlugin::new(parent))
    }
}
