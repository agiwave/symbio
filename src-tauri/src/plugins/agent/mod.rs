//! Agent 插件模块
//!
//! Agent 是通用的插件容器，内置 add/list/remove 三个管理插件

mod add;
mod remove;
mod factory;

pub use add::AddPlugin;
pub use remove::RemovePlugin;
pub use factory::AgentFactory;

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult};
use crate::core::PluginFactoryRegistry;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

pub struct Agent {
    meta: PluginMeta,
    instances: HashMap<String, Arc<dyn Plugin>>,
}

impl Agent {
    pub fn new() -> Self {
        let meta = PluginMeta {
            name: "agent".to_string(),
            description: "通用的插件容器，可以管理子插件实例".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: Some(json!({
                "type": "object",
                "properties": {
                    "plugins": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "子插件列表"
                    },
                    "message": {
                        "type": "string",
                        "description": "状态消息"
                    }
                }
            })),
            author: Some("Symbio Team".to_string()),
        };

        let mut agent = Agent {
            meta,
            instances: HashMap::new(),
        };

        agent.init_builtin_plugins();
        agent
    }

    fn init_builtin_plugins(&mut self) {
        let registry = PluginFactoryRegistry::global();
        
        // 注册内置管理插件：add, list, remove
        self.instances.insert("add".to_string(), Arc::new(AddPlugin::new()));
        self.instances.insert("remove".to_string(), Arc::new(RemovePlugin::new()));

        // 注册工厂插件（跳过 agent 自身，避免无限递归）
        for factory in registry.list() {
            let name = factory.meta().name.clone();
            if name == "agent" {
                continue; // agent 不自动创建自己作为子插件
            }
            let plugin = factory.create(Some(&*self), None);
            self.instances.insert(name, plugin);
        }
    }

    /// 获取所有插件名称列表
    pub fn list_plugins(&self) -> Vec<String> {
        let mut names: Vec<String> = self.instances.keys().cloned().collect();
        names.sort();
        names
    }

    /// 添加插件（由命令层调用）
    pub fn add_plugin(&mut self, name: &str, config: Option<Value>) -> PluginResult<()> {
        let registry = PluginFactoryRegistry::global();
        let factory = registry.list()
            .into_iter()
            .find(|f| f.meta().name == name)
            .ok_or_else(|| crate::core::types::PluginError::NotFound(format!("工厂 '{}' 未找到", name)))?;
        let plugin = factory.create(Some(&*self), config.as_ref());
        self.instances.insert(name.to_string(), plugin);
        Ok(())
    }

    /// 删除插件（由命令层调用）
    pub fn remove_plugin(&mut self, name: &str) -> PluginResult<()> {
        if name == "add" || name == "list" || name == "remove" {
            return Err(crate::core::types::PluginError::ValidationError("不能删除内置管理插件".to_string()));
        }
        self.instances.remove(name)
            .ok_or_else(|| crate::core::types::PluginError::NotFound(format!("插件 '{}' 未找到", name)))?;
        Ok(())
    }

    /// 获取所有插件信息（由 ListPlugin 调用）
    pub fn get_plugins_info(&self) -> Vec<Value> {
        self.instances.iter().map(|(name, plugin)| {
            let meta = plugin.meta();
            json!({
                "name": name,
                "description": meta.description,
                "version": meta.version,
                "input_schema": meta.input,
                "output_schema": meta.output
            })
        }).collect()
    }

    pub fn get_plugin(&self, name: &str) -> Option<Arc<dyn Plugin>> {
        self.instances.get(name).cloned()
    }
}

impl Default for Agent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Plugin for Agent {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }

    async fn invoke(&self, _input: Value) -> PluginResult<Value> {
        Ok(Value::Object(serde_json::Map::from_iter([
            ("plugins".to_string(), Value::Array(
                self.instances.keys()
                    .map(|n| Value::String(n.clone()))
                    .collect()
            )),
            ("message".to_string(), Value::String("Agent 插件就绪".to_string())),
        ])))
    }

    fn plugin(&self, path: &[String]) -> Option<Arc<dyn Plugin>> {
        if path.is_empty() {
            return None;
        }

        let mut current: Option<Arc<dyn Plugin>> = None;

        for (i, name) in path.iter().enumerate() {
            current = if i == 0 {
                self.instances.get(name).cloned()
            } else {
                current?.plugin(&path[i..])
            };
        }

        current
    }
}
