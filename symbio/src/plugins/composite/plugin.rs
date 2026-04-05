//! Composite 插件实现
//!
//! 通用组合插件，支持动态管理多个子插件

use crate::symbio_core::traits::Plugin;
use crate::symbio_core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use crate::symbio_core::registry::PluginFactoryRegistry;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};
use indexmap::IndexMap;

/// 子插件配置
#[derive(Debug, Clone)]
pub struct SubPluginConfig {
    /// 子插件名称（在组合中的标识）
    pub name: String,
    /// 工厂名称（用于创建实例）
    pub factory: String,
    /// 插件配置（传递给工厂）
    pub config: Option<Value>,
}

/// Composite 插件元数据配置
#[derive(Debug, Clone)]
pub struct CompositeMetaConfig {
    /// 插件名称
    pub name: String,
    /// 插件标题（用于显示）
        pub title: String,
    /// 插件描述
    pub description: String,
    /// 版本号
    pub version: String,
    /// 作者
    pub author: Option<String>,
}

impl Default for CompositeMetaConfig {
    fn default() -> Self {
        CompositeMetaConfig {
            name: "composite".to_string(),
            title: "Composite Plugin".to_string(),
            description: "通用组合插件".to_string(),
            version: "0.1.0".to_string(),
            author: Some("Symbio Team".to_string()),
        }
    }
}

/// Composite 插件
pub struct CompositePlugin {
    meta: PluginMeta,
    plugins: IndexMap<String, Arc<dyn Plugin>>,
    /// 自身引用，用于传递给动态创建的子插件
    self_ref: Option<Weak<dyn Plugin>>,
}

impl CompositePlugin {
    /// 创建新的 Composite 插件
    pub fn new(meta_config: CompositeMetaConfig, sub_plugins: IndexMap<String, Arc<dyn Plugin>>) -> Self {
        let input_schema = json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "add", "remove", "invoke"],
                    "description": "操作类型"
                },
                "plugin_name": {
                    "type": "string",
                    "description": "子插件名称（add/remove/invoke 时需要）"
                },
                "factory": {
                    "type": "string",
                    "description": "工厂名称（add 时需要）"
                },
                "config": {
                    "type": "object",
                    "description": "插件配置（add 时需要）"
                },
                "input": {
                    "type": "object",
                    "description": "调用输入（invoke 时需要）"
                }
            }
        });

        let output_schema = json!({
            "type": "object",
            "properties": {
                "success": {
                    "type": "boolean"
                },
                "data": {
                    "type": "object"
                },
                "error": {
                    "type": "string"
                }
            }
        });

        CompositePlugin {
            meta: PluginMeta {
                name: meta_config.name,
                description: meta_config.description,
                version: meta_config.version,
                input: Some(input_schema),
                output: Some(output_schema),
                author: meta_config.author,
            },
            plugins: sub_plugins,
            self_ref: None,
        }
    }

    /// 创建 Arc 包装的 Composite 插件（支持自身引用）
    pub fn new_arc(meta_config: CompositeMetaConfig, sub_plugins: IndexMap<String, Arc<dyn Plugin>>) -> Arc<Self> {
        Arc::new_cyclic(|weak| {
            let input_schema = json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "add", "remove", "invoke"],
                        "description": "操作类型"
                    },
                    "plugin_name": {
                        "type": "string",
                        "description": "子插件名称（add/remove/invoke 时需要）"
                    },
                    "factory": {
                        "type": "string",
                        "description": "工厂名称（add 时需要）"
                    },
                    "config": {
                        "type": "object",
                        "description": "插件配置（add 时需要）"
                    },
                    "input": {
                        "type": "object",
                        "description": "调用输入（invoke 时需要）"
                    }
                }
            });

            let output_schema = json!({
                "type": "object",
                "properties": {
                    "success": {
                        "type": "boolean"
                    },
                    "data": {
                        "type": "object"
                    },
                    "error": {
                        "type": "string"
                    }
                }
            });

            CompositePlugin {
                meta: PluginMeta {
                    name: meta_config.name,
                    description: meta_config.description,
                    version: meta_config.version,
                    input: Some(input_schema),
                    output: Some(output_schema),
                    author: meta_config.author,
                },
                plugins: sub_plugins,
                self_ref: Some(weak.clone() as Weak<dyn Plugin>),
            }
        })
    }

    /// 设置自身引用（在 Arc 创建后调用）
    pub fn set_self_ref(&mut self, weak: Option<Weak<dyn Plugin>>) {
        self.self_ref = weak;
    }

    /// 获取所有子插件名称列表
        pub fn list_plugins(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    /// 添加子插件
        pub fn add_plugin(&mut self, name: String, plugin: Arc<dyn Plugin>) {
        self.plugins.insert(name, plugin);
    }

    /// 移除子插件
        pub fn remove_plugin(&mut self, name: &str) -> Option<Arc<dyn Plugin>> {
        self.plugins.shift_remove(name)
    }

    /// 获取子插件
        pub fn get_plugin(&self, name: &str) -> Option<Arc<dyn Plugin>> {
        self.plugins.get(name).cloned()
    }

    /// 通过工厂创建子插件
        pub fn create_sub_plugin(
        &self,
        factory_name: &str,
        config: Option<&Value>,
    ) -> Result<Arc<dyn Plugin>, PluginError> {
        let registry = PluginFactoryRegistry::global();
        let factories = registry.list();
        
        let factory = factories.iter()
            .find(|f| f.meta().name == factory_name)
            .ok_or_else(|| PluginError::NotFound(format!("工厂 '{}' 未找到", factory_name)))?;

        // 传递自身弱引用给子插件
        let parent = self.self_ref.as_ref().map(|w| w.clone());
        Ok(factory.create(parent, config))
    }

    /// 路由到子插件
    fn route(&self, path: &str) -> Result<(Arc<dyn Plugin>, String), PluginError> {
        if path.is_empty() {
            return Err(PluginError::NotFound("路径为空".to_string()));
        }

        let parts: Vec<&str> = path.splitn(2, '/').collect();
        let plugin_name = parts[0];
        let sub_path = parts.get(1).map(|s| s.to_string()).unwrap_or_default();

        self.plugins.get(plugin_name)
            .map(|p| (Arc::clone(p), sub_path))
            .ok_or_else(|| PluginError::NotFound(format!("子插件 '{}' 未找到", plugin_name)))
    }

    /// 处理管理操作（list/add/remove）
    fn handle_admin_action(&self, action: &str, _input: &Value) -> Result<Value, PluginError> {
        match action {
            "list" => {
                let plugins: Vec<Value> = self.plugins.iter().map(|(name, plugin)| {
                    json!({
                        "name": name,
                        "meta": plugin.meta("").ok().map(|m| json!({
                            "title": m.name,
                            "description": m.description,
                            "version": m.version
                        }))
                    })
                }).collect();
                Ok(json!({ "plugins": plugins }))
            }
            _ => Err(PluginError::ValidationError(format!("未知的管理操作：{}", action)))
        }
    }
}

#[async_trait::async_trait]
impl Plugin for CompositePlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            return Ok(self.meta.clone());
        }
        
        // 路由到子插件
        let (plugin, sub_path) = self.route(path)?;
        plugin.meta(&sub_path)
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if path.is_empty() {
            // 处理 Composite 自身的管理操作
            let obj = input.as_object()
                .ok_or_else(|| PluginError::ValidationError("输入必须是对象".to_string()))?;

            let action = obj.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("list");

            match action {
                "list" => {
                    let result = self.handle_admin_action(action, &input)?;
                    return Ok(InvokeStream::single(json!({
                        "success": true,
                        "data": result
                    })));
                }
                "add" => {
                    // 添加子插件需要修改状态，这里返回错误提示
                    return Ok(InvokeStream::single(json!({
                        "success": false,
                        "error": "add 操作需要通过可变引用调用，建议使用外部管理接口"
                    })));
                }
                "remove" => {
                    // 移除子插件需要修改状态
                    return Ok(InvokeStream::single(json!({
                        "success": false,
                        "error": "remove 操作需要通过可变引用调用，建议使用外部管理接口"
                    })));
                }
                "invoke" => {
                    let plugin_name = obj.get("plugin_name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| PluginError::ValidationError("invoke 操作需要 plugin_name 参数".to_string()))?;
                    
                    let sub_input = obj.get("input")
                        .cloned()
                        .unwrap_or(json!({}));

                    let plugin = self.plugins.get(plugin_name)
                        .ok_or_else(|| PluginError::NotFound(format!("子插件 '{}' 未找到", plugin_name)))?;

                    return plugin.invoke("", sub_input);
                }
                _ => {
                    return Err(PluginError::ValidationError(format!("未知的操作：{}", action)));
                }
            }
        }

        // 路由到子插件
        let (plugin, sub_path) = self.route(path)?;
        plugin.invoke(&sub_path, input)
    }
}
