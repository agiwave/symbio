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

// 导出子插件工厂
pub use chat::factory::ChatFactory;
pub use tools::factory::ToolsFactory;
pub use memory::factory::MemoryFactory;
pub use session::factory::SessionFactory;
pub use telegram::factory::TelegramFactory;
pub use openai::factory::OpenAiFactory;
pub use factory::AgentFactory;

use crate::symbio_core::traits::Plugin;
use crate::symbio_core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Weak, RwLock};

pub struct Agent {
    meta: PluginMeta,
    instances: RwLock<HashMap<String, Arc<dyn Plugin>>>,
    /// 父插件引用（用于转发 save_config 等请求）
    parent: RwLock<Option<Weak<dyn Plugin>>>,
}

impl Agent {
    fn create_meta() -> PluginMeta {
        PluginMeta {
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
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new() -> Self {
        Self {
            meta: Self::create_meta(),
            instances: RwLock::new(HashMap::new()),
            parent: RwLock::new(None),
        }
    }

    /// 添加子插件实例
    pub fn add_instance(&self, name: String, plugin: Arc<dyn Plugin>) {
        self.instances.write().unwrap().insert(name, plugin);
    }

    /// 设置父引用
    pub fn set_parent(&self, parent: Weak<dyn Plugin>) {
        *self.parent.write().unwrap() = Some(parent);
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.read().unwrap().as_ref().and_then(|w| w.upgrade())
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
        let instances = self.instances.read().unwrap();
        for plugin in instances.values() {
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
        
        let instances = self.instances.read().unwrap();
        let plugin = instances.get(name)
            .ok_or_else(|| PluginError::NotFound(format!("插件 '{}' 未找到", name)))?;
        
        plugin.meta(rest)
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 绝对路径（以 / 开头）：转发给父插件处理
        if path.starts_with('/') {
            if let Some(parent) = self.get_parent() {
                eprintln!("[agent] forwarding absolute path '{}' to parent", path);
                return parent.invoke(path, input);
            } else {
                return Err(PluginError::NotFound(format!("无法解析绝对路径 '{}'：没有父插件", path)));
            }
        }

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
            let instances = self.instances.read().unwrap();
            let mut capabilities: Vec<Value> = Vec::new();
            for (name, plugin) in instances.iter() {
                let caps: Vec<&str> = plugin.capabilities();
                capabilities.push(json!({
                    "plugin": name,
                    "capabilities": caps
                }));
            }
            
            return Ok(InvokeStream::single(json!({
                "success": true,
                "data": {
                    "plugins": instances.keys().cloned().collect::<Vec<_>>(),
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
                            let instances = self.instances.read().unwrap();
                            let mut configs = serde_json::Map::new();
                            for (name, plugin) in instances.iter() {
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
                                    let instances = self.instances.read().unwrap();
                                    for (name, plugin_config) in obj {
                                        // 跳过非对象字段（如 success）
                                        if !plugin_config.is_object() {
                                            continue;
                                        }
                                        if let Some(plugin) = instances.get(name) {
                                            eprintln!("[agent] distributing config to '{}': {:?}", name, plugin_config);
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

        // available_tools path: 递归收集所有子插件的可用工具
        if path == "available_tools" {
            let tools = self.available_tools();
            return Ok(InvokeStream::single(json!({
                "success": true,
                "tools": tools
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
        
        let instances = self.instances.read().unwrap();
        let plugin = instances.get(name)
            .ok_or_else(|| PluginError::NotFound(format!("插件 '{}' 未找到", name)))?;
        
        plugin.invoke(rest, input)
    }

    fn capabilities(&self) -> Vec<&'static str> {
        vec!["agent"]
    }

    fn available_tools(&self) -> Vec<PluginMeta> {
        let instances = self.instances.read().unwrap();
        let mut all_tools = Vec::new();

        // 递归收集所有子插件的工具，并根据子插件实例名添加前缀
        for (name, plugin) in instances.iter() {
            let tools = plugin.available_tools();
            for mut tool in tools {
                // 根据子插件实例名添加前缀（如 tools/read_file, memory/store）
                tool.name = format!("{}/{}", name, tool.name);
                all_tools.push(tool);
            }
        }

        all_tools
    }
}