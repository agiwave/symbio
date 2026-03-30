//! Composite 插件工厂
//!
//! 通过配置创建 Composite 插件实例，支持动态组装子插件

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::{CompositePlugin, CompositeMetaConfig, SubPluginConfig};
use serde_json::Value;
use std::sync::Arc;
use indexmap::IndexMap;

/// Composite 插件工厂配置
#[derive(Debug, Clone)]
pub struct CompositeFactoryConfig {
    /// 元数据配置
    pub meta: CompositeMetaConfig,
    /// 子插件配置列表
    pub plugins: Vec<SubPluginConfig>,
}

impl Default for CompositeFactoryConfig {
    fn default() -> Self {
        CompositeFactoryConfig {
            meta: CompositeMetaConfig::default(),
            plugins: Vec::new(),
        }
    }
}

pub struct CompositeFactory {
    config: CompositeFactoryConfig,
}

impl CompositeFactory {
    /// 创建新的 Composite 工厂
    pub fn new(config: CompositeFactoryConfig) -> Self {
        CompositeFactory { config }
    }

    /// 使用默认配置创建工厂
    pub fn with_defaults() -> Self {
        CompositeFactory::new(CompositeFactoryConfig::default())
    }

    /// 添加子插件配置
    #[allow(dead_code)]
    pub fn add_sub_plugin_config(mut self, name: String, factory: String, config: Option<Value>) -> Self {
        self.config.plugins.push(SubPluginConfig {
            name,
            factory,
            config,
        });
        self
    }

    /// 设置元数据配置
    #[allow(dead_code)]
    pub fn with_meta_config(mut self, meta: CompositeMetaConfig) -> Self {
        self.config.meta = meta;
        self
    }
}

#[async_trait::async_trait]
impl PluginFactory for CompositeFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: self.config.meta.name.clone(),
            description: self.config.meta.description.clone(),
            version: self.config.meta.version.clone(),
            input: None,
            output: None,
            author: self.config.meta.author.clone(),
        }
    }

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        let mut plugins = IndexMap::new();

        // 通过工厂创建每个子插件
        for sub_config in &self.config.plugins {
            // 使用全局注册表获取工厂
            let registry = crate::core::registry::PluginFactoryRegistry::global();
            let factories = registry.list();
            if let Some(factory) = factories.iter().find(|f| f.meta().name == sub_config.factory) {
                let plugin = factory.create(None, sub_config.config.as_ref());
                plugins.insert(sub_config.name.clone(), plugin);
            }
        }

        Arc::new(CompositePlugin::new(
            self.config.meta.clone(),
            plugins,
        ))
    }
}
