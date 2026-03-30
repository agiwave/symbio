//! Agent 插件模块
//!
//! Agent 是通用的插件容器，内置 add/remove/chat/tools/memory 子插件

mod add;
mod remove;
mod chat;
mod tools;
mod memory;
mod factory;

pub use add::AddPlugin;
pub use remove::RemovePlugin;
pub use chat::ChatPlugin;
pub use chat::factory::ChatFactory;
pub use tools::ToolsPlugin;
pub use tools::factory::ToolsFactory;
pub use memory::MemoryPlugin;
pub use memory::factory::MemoryFactory;
pub use factory::AgentFactory;

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
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
        
        // 注册内置管理插件：add, remove
        self.instances.insert("add".to_string(), Arc::new(AddPlugin::new()));
        self.instances.insert("remove".to_string(), Arc::new(RemovePlugin::new()));
        
        // 注册 chat 插件
        self.instances.insert("chat".to_string(), Arc::new(ChatPlugin::new()));
        
        // 注册 tools 插件
        self.instances.insert("tools".to_string(), Arc::new(ToolsPlugin::default()));
        
        // 注册 memory 插件
        self.instances.insert("memory".to_string(), Arc::new(MemoryPlugin::default()));

        // 注册工厂插件（跳过 agent 自身，避免无限递归）
        for factory in registry.list() {
            let name = factory.meta().name.clone();
            if name == "agent" || name == "chat" || name == "tools" || name == "memory" {
                continue;
            }
            let plugin = factory.create(Some(&*self), None);
            self.instances.insert(name, plugin);
        }
    }

    /// 解析路径，返回 (子插件名, 剩余路径)
    fn parse_path(path: &str) -> Option<(&str, &str)> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return None;
        }
        match path.find('/') {
            Some(idx) => Some((&path[..idx], &path[idx + 1..])),
            None => Some((path, "")),
        }
    }
}

impl Default for Agent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Plugin for Agent {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            return Ok(self.meta.clone());
        }
        
        let (name, rest) = Self::parse_path(path)
            .ok_or_else(|| PluginError::NotFound(format!("插件路径 '{}' 未找到", path)))?;
        
        let plugin = self.instances.get(name)
            .ok_or_else(|| PluginError::NotFound(format!("插件 '{}' 未找到", name)))?;
        
        plugin.meta(rest)
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if path.is_empty() {
            return Ok(InvokeStream::single(Value::Object(serde_json::Map::from_iter([
                ("plugins".to_string(), Value::Array(
                    self.instances.keys()
                        .map(|n| Value::String(n.clone()))
                        .collect()
                )),
                ("message".to_string(), Value::String("Agent 插件就绪".to_string())),
            ]))));
        }
        
        let (name, rest) = Self::parse_path(path)
            .ok_or_else(|| PluginError::NotFound(format!("插件路径 '{}' 未找到", path)))?;
        
        let plugin = self.instances.get(name)
            .ok_or_else(|| PluginError::NotFound(format!("插件 '{}' 未找到", name)))?;
        
        plugin.invoke(rest, input)
    }
}
