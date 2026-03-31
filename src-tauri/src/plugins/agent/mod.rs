//! Agent 插件模块
//!
//! Agent 是通用的插件容器，支持：
//! - 子插件管理
//! - 能力路由 (@llm, @session 等)

mod add;
mod remove;
mod chat;
mod tools;
mod memory;
mod session;
mod telegram;
mod openai;
mod factory;

pub use add::AddPlugin;
pub use remove::RemovePlugin;
pub use chat::ChatPlugin;
pub use chat::factory::ChatFactory;
pub use tools::ToolsPlugin;
pub use tools::factory::ToolsFactory;
pub use memory::MemoryPlugin;
pub use memory::factory::MemoryFactory;
pub use session::SessionPlugin;
pub use session::factory::SessionFactory;
pub use telegram::TelegramPlugin;
pub use telegram::factory::TelegramFactory;
pub use openai::OpenAiPlugin;
pub use openai::OpenAiFactory;
pub use factory::AgentFactory;

// 导入 docker 插件（来自 plugins/docker）
use crate::plugins::docker::DockerPlugin;

use crate::core::traits::{Plugin, ParentRef};
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use crate::core::PluginFactoryRegistry;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Weak};

pub struct Agent {
    meta: PluginMeta,
    instances: HashMap<String, Arc<dyn Plugin>>,
    /// 自身引用，用于传递给子插件
    self_ref: Option<Weak<dyn Plugin>>,
    /// 父插件引用（用于转发 save_config 等请求）
    parent: Option<Weak<dyn Plugin>>,
}

impl Agent {
    /// 创建 Agent 实例（返回 Arc 包装）
    /// 
    /// 使用 Arc::new_cyclic 创建自引用
    pub fn new_arc() -> Arc<Self> {
        Arc::new_cyclic(|weak| {
            Agent {
                meta: PluginMeta {
                    name: "agent".to_string(),
                    description: "通用的插件容器，支持能力路由".to_string(),
                    version: "0.1.0".to_string(),
                    input: None,
                    output: Some(json!({
                        "type": "object",
                        "properties": {
                            "plugins": {
                                "type": "array",
                                "items": {"type": "string"}
                            },
                            "message": { "type": "string" }
                        }
                    })),
                    author: Some("Symbio Team".to_string()),
                },
                instances: HashMap::new(),
                self_ref: Some(weak.clone() as Weak<dyn Plugin>),
                parent: None,
            }
        })
    }

    /// 初始化内置插件（消费并返回新的 Arc）
    pub fn init_builtin_plugins(self: Arc<Self>) -> Arc<Self> {
        let registry = PluginFactoryRegistry::global();
        let parent: Option<Arc<dyn Plugin>> = Some(Arc::clone(&self) as Arc<dyn Plugin>);
        
        let mut instances = HashMap::new();
        
        // 注册内置管理插件（不需要父引用）
        instances.insert("add".to_string(), Arc::new(AddPlugin::new()) as Arc<dyn Plugin>);
        instances.insert("remove".to_string(), Arc::new(RemovePlugin::new()) as Arc<dyn Plugin>);
        
        // 注册 chat 插件（需要父引用来调用 @llm）
        instances.insert("chat".to_string(), ChatPlugin::with_parent(parent.clone()));
        
        // 其他插件
        instances.insert("tools".to_string(), Arc::new(ToolsPlugin::default()) as Arc<dyn Plugin>);
        instances.insert("memory".to_string(), Arc::new(MemoryPlugin::default()) as Arc<dyn Plugin>);
        instances.insert("session".to_string(), Arc::new(SessionPlugin::default()) as Arc<dyn Plugin>);
        instances.insert("telegram".to_string(), Arc::new(TelegramPlugin::default()) as Arc<dyn Plugin>);
        instances.insert("openai".to_string(), OpenAiPlugin::with_parent(parent.clone(), Default::default()));
        instances.insert("docker".to_string(), Arc::new(DockerPlugin::new()) as Arc<dyn Plugin>);

        // 注册工厂插件
        for factory in registry.list() {
            let name = factory.meta().name.clone();
            if instances.contains_key(&name) {
                continue;
            }
            let plugin = factory.create(parent.clone(), None);
            instances.insert(name, plugin);
        }
        
        // 尝试解包 Arc，如果成功则修改后重新包装
        match Arc::try_unwrap(self) {
            Ok(mut agent) => {
                agent.instances = instances;
                Arc::new(agent)
            }
            Err(arc) => {
                // 如果有其他引用，使用 get_mut（需要可变引用）
                // 这种情况不应该发生在初始化阶段
                arc
            }
        }
    }

    /// 创建 Agent 实例
    pub fn new() -> Self {
        Self {
            meta: PluginMeta {
                name: "agent".to_string(),
                description: "通用的插件容器，支持能力路由".to_string(),
                version: "0.1.0".to_string(),
                input: None,
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "plugins": { "type": "array", "items": {"type": "string"} },
                        "message": { "type": "string" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            instances: HashMap::new(),
            self_ref: None,
            parent: None,
        }
    }

    /// 创建带父引用的 Agent 实例
    pub fn with_parent(parent: Option<Arc<dyn Plugin>>) -> Self {
        Self {
            meta: PluginMeta {
                name: "agent".to_string(),
                description: "通用的插件容器，支持能力路由".to_string(),
                version: "0.1.0".to_string(),
                input: None,
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "plugins": { "type": "array", "items": {"type": "string"} },
                        "message": { "type": "string" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            instances: HashMap::new(),
            self_ref: None,
            parent: parent.map(|p| Arc::downgrade(&p)),
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
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

    /// 检查是否是能力路由（以 @ 开头）
    fn is_capability_route(path: &str) -> bool {
        path.starts_with('@')
    }

    /// 根据能力查找插件
    fn find_by_capability(&self, capability: &str) -> Option<Arc<dyn Plugin>> {
        for plugin in self.instances.values() {
            if plugin.capabilities().iter().any(|c| *c == capability) {
                return Some(Arc::clone(plugin));
            }
        }
        None
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

        // config path
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Agent 配置管理（收集所有子插件配置）".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["get", "set"],
                            "description": "get获取所有子插件配置，set分发配置到子插件"
                        },
                        "config": {
                            "type": "object",
                            "description": "配置数据（各子插件的配置）"
                        }
                    },
                    "required": ["action"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "config": { "type": "object" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            });
        }
        
        // 能力路由
        if Self::is_capability_route(path) {
            let capability = path.trim_start_matches('@');
            // 移除可能的后缀路径
            let capability = capability.split('/').next().unwrap_or(capability);
            
            if let Some(plugin) = self.find_by_capability(capability) {
                return plugin.meta("");
            }
            return Err(PluginError::NotFound(format!("能力 '{}' 未找到", capability)));
        }
        
        let (name, rest) = Self::parse_path(path)
            .ok_or_else(|| PluginError::NotFound(format!("插件路径 '{}' 未找到", path)))?;
        
        let plugin = self.instances.get(name)
            .ok_or_else(|| PluginError::NotFound(format!("插件 '{}' 未找到", name)))?;
        
        plugin.meta(rest)
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 转发配置保存/加载请求到父插件（Home）
        if path == "save_config" || path == "load_config" {
            eprintln!("[agent] received {} request", path);
            if let Some(parent) = self.get_parent() {
                eprintln!("[agent] forwarding {} to parent", path);
                return parent.invoke(path, input);
            } else {
                eprintln!("[agent] ERROR: no parent for {}", path);
                return Ok(InvokeStream::single(json!({
                    "success": false,
                    "error": "无法保存配置：没有父插件"
                })));
            }
        }

        if path.is_empty() {
            let mut capabilities: Vec<Value> = Vec::new();
            for (name, plugin) in self.instances.iter() {
                let caps: Vec<&str> = plugin.capabilities();
                capabilities.push(json!({
                    "plugin": name,
                    "capabilities": caps
                }));
            }
            
            return Ok(InvokeStream::single(json!({
                "success": true,
                "data": {
                    "plugins": self.instances.keys().cloned().collect::<Vec<_>>(),
                    "capabilities": capabilities,
                    "message": "Agent 插件就绪"
                }
            })));
        }

        // config path: 收集/分发子插件配置
        if path == "config" {
            let action = input.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get");

            return Ok(InvokeStream::Single(tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    match action {
                        "get" => {
                            let mut configs = serde_json::Map::new();
                            for (name, plugin) in &self.instances {
                                if let Ok(InvokeStream::Single(chunk)) = plugin.invoke("config", json!({"action": "get"})) {
                                    if chunk.error.is_none() && !chunk.data.is_null() {
                                        configs.insert(name.clone(), chunk.data);
                                    }
                                }
                            }
                            StreamChunk {
                                data: json!({ "success": true, "config": configs }),
                                done: true,
                                error: None,
                            }
                        }
                        "set" => {
                            if let Some(config) = input.get("config") {
                                if let Some(obj) = config.as_object() {
                                    for (name, plugin_config) in obj {
                                        if let Some(plugin) = self.instances.get(name) {
                                            let _ = plugin.invoke("config", json!({
                                                "action": "set",
                                                "config": plugin_config
                                            }));
                                        }
                                    }
                                }
                            }
                            StreamChunk {
                                data: json!({ "success": true }),
                                done: true,
                                error: None,
                            }
                        }
                        _ => StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(format!("未知操作: {}", action)),
                        },
                    }
                })
            })));
        }
        
        // 能力路由：@llm, @session 等
        if Self::is_capability_route(path) {
            let (capability, rest) = match path.find('/') {
                Some(idx) => (&path[1..idx], &path[idx + 1..]),
                None => (&path[1..], ""),
            };
            eprintln!("[agent] capability route: capability='{}', rest='{}'", capability, rest);
            
            let plugin = self.find_by_capability(capability)
                .ok_or_else(|| {
                    eprintln!("[agent] ERROR: capability '{}' not found", capability);
                    PluginError::NotFound(format!("能力 '{}' 未找到", capability))
                })?;
            
            eprintln!("[agent] found plugin for capability '{}', invoking with rest='{}'", capability, rest);
            return plugin.invoke(rest, input);
        }
        
        // 普通路径路由
        let (name, rest) = Self::parse_path(path)
            .ok_or_else(|| PluginError::NotFound(format!("插件路径 '{}' 未找到", path)))?;
        
        let plugin = self.instances.get(name)
            .ok_or_else(|| PluginError::NotFound(format!("插件 '{}' 未找到", name)))?;
        
        plugin.invoke(rest, input)
    }

    fn capabilities(&self) -> Vec<&'static str> {
        vec!["agent"]
    }
}