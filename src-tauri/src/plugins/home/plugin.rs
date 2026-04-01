//! Home 插件 - 根插件，持有 work/agent/setting 子插件实例
//!
//! 职责：
//! - 子插件路由
//! - 全局配置管理（~/.symbio/config.yaml）

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 配置文件路径
fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".symbio")
        .join("config.yaml")
}

/// 全局配置结构
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct GlobalConfig {
    #[serde(default)]
    pub plugins: serde_json::Map<String, Value>,
}

pub struct HomePlugin {
    meta: PluginMeta,
    work: Arc<dyn Plugin>,
    agent: Arc<dyn Plugin>,
    setting: Arc<dyn Plugin>,
    config: Arc<RwLock<GlobalConfig>>,
}

impl HomePlugin {
    pub fn new(
        work: Arc<dyn Plugin>,
        agent: Arc<dyn Plugin>,
        setting: Arc<dyn Plugin>,
    ) -> Self {
        HomePlugin {
            meta: PluginMeta {
                name: "home".to_string(),
                description: "Symbio 主插件".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "子插件路径，如 work/agent/setting"
                        }
                    }
                })),
                output: None,
                author: Some("Symbio Team".to_string()),
            },
            work,
            agent,
            setting,
            config: Arc::new(RwLock::new(GlobalConfig::default())),
        }
    }

    /// 从配置文件加载配置
    pub async fn load_config(&self) -> Result<(), PluginError> {
        let path = config_path();
        if !path.exists() {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| PluginError::InternalError(format!("读取配置失败: {}", e)))?;

        let config: GlobalConfig = serde_yaml::from_str(&content)
            .map_err(|e| PluginError::ParseError(format!("解析配置失败: {}", e)))?;

        let mut cfg = self.config.write().await;
        *cfg = config;

        Ok(())
    }

    /// 保存配置到文件
    pub async fn save_config(&self) -> Result<(), PluginError> {
        eprintln!("[home] save_config called");
        
        // 先收集所有子插件配置
        let collected = self.collect_plugin_configs().await;
        eprintln!("[home] collected configs: {:?}", collected);
        
        {
            let mut cfg = self.config.write().await;
            cfg.plugins = collected;
        }

        let path = config_path();
        eprintln!("[home] config path: {:?}", path);

        // 确保目录存在
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| PluginError::InternalError(format!("创建目录失败: {}", e)))?;
        }

        let cfg = self.config.read().await;
        let content = serde_yaml::to_string(&*cfg)
            .map_err(|e| PluginError::InternalError(format!("序列化配置失败: {}", e)))?;

        tokio::fs::write(&path, content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入配置失败: {}", e)))?;

        eprintln!("[home] config saved successfully");
        Ok(())
    }

    /// 收集所有子插件配置
    async fn collect_plugin_configs(&self) -> serde_json::Map<String, Value> {
        let mut configs = serde_json::Map::new();

        // 收集 work 配置（work 直接返回配置对象，不需要提取）
        if let Ok(InvokeStream::Single(chunk)) = self.work.invoke("config", json!({"action": "get"})) {
            if chunk.error.is_none() && !chunk.data.is_null() {
                configs.insert("work".to_string(), chunk.data);
            }
        }

        // 收集 agent 配置（agent 返回 { "success": true, "config": {...} }，需要提取 config）
        if let Ok(InvokeStream::Single(chunk)) = self.agent.invoke("config", json!({"action": "get"})) {
            if chunk.error.is_none() && !chunk.data.is_null() {
                // 提取内部的 config 对象
                if let Some(agent_config) = chunk.data.get("config") {
                    configs.insert("agent".to_string(), agent_config.clone());
                } else {
                    configs.insert("agent".to_string(), chunk.data);
                }
            }
        }

        // 收集 setting 配置
        if let Ok(InvokeStream::Single(chunk)) = self.setting.invoke("config", json!({"action": "get"})) {
            if chunk.error.is_none() && !chunk.data.is_null() {
                // setting 可能也返回 { "success": true, "data": {...} }
                if let Some(setting_data) = chunk.data.get("data") {
                    configs.insert("setting".to_string(), setting_data.clone());
                } else {
                    configs.insert("setting".to_string(), chunk.data);
                }
            }
        }

        configs
    }

    /// 分发配置到各子插件
    async fn distribute_configs(&self, configs: &serde_json::Map<String, Value>) {
        // 分发 work 配置
        if let Some(work_config) = configs.get("work") {
            let _ = self.work.invoke("config", json!({
                "action": "set",
                "config": work_config
            }));
        }

        // 分发 agent 配置
        if let Some(agent_config) = configs.get("agent") {
            let _ = self.agent.invoke("config", json!({
                "action": "set",
                "config": agent_config
            }));
        }

        // 分发 setting 配置
        if let Some(setting_config) = configs.get("setting") {
            let _ = self.setting.invoke("config", json!({
                "action": "set",
                "config": setting_config
            }));
        }
    }

    fn route(&self, path: &str) -> Result<(Arc<dyn Plugin>, String), PluginError> {
        let parts: Vec<&str> = path.splitn(2, '/').collect();
        let plugin_name = parts[0];
        let sub_path = parts.get(1).map(|s| s.to_string()).unwrap_or_default();

        match plugin_name {
            "work" => Ok((Arc::clone(&self.work), sub_path)),
            "agent" => Ok((Arc::clone(&self.agent), sub_path)),
            "setting" => Ok((Arc::clone(&self.setting), sub_path)),
            _ => Err(PluginError::NotFound(format!("未知的插件路径: {}", plugin_name))),
        }
    }

    fn handle_config_meta(&self) -> PluginMeta {
        PluginMeta {
            name: "config".to_string(),
            description: "全局配置管理".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "set", "save", "load", "collect"],
                        "description": "操作类型：get获取配置，set设置配置，save保存到文件，load从文件加载，collect收集所有子插件配置"
                    },
                    "config": {
                        "type": "object",
                        "description": "配置数据（用于set操作）"
                    }
                },
                "required": ["action"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "config": { "type": "object" },
                    "error": { "type": "string" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }
}

#[async_trait::async_trait]
impl Plugin for HomePlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            return Ok(self.meta.clone());
        }
        if path == "config" {
            return Ok(self.handle_config_meta());
        }
        let (plugin, sub_path) = self.route(path)?;
        plugin.meta(&sub_path)
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        eprintln!("[home] invoke called with path: '{}'", path);
        
        // 处理 save_config 和 load_config（子插件通过 parent 链调用）
        if path == "save_config" {
            eprintln!("[home] handling save_config");
            let home_self = Arc::new(self.clone());
            return Ok(InvokeStream::Single(tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    match home_self.save_config().await {
                        Ok(()) => StreamChunk {
                            data: json!({ "success": true, "message": "配置已保存" }),
                            done: true,
                            error: None,
                        },
                        Err(e) => StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(e.to_string()),
                        },
                    }
                })
            })));
        }

        if path == "load_config" {
            let home_self = Arc::new(self.clone());
            return Ok(InvokeStream::Single(tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    match home_self.load_config().await {
                        Ok(()) => {
                            let cfg = home_self.config.read().await;
                            home_self.distribute_configs(&cfg.plugins).await;
                            StreamChunk {
                                data: json!({ "success": true, "config": &cfg.plugins }),
                                done: true,
                                error: None,
                            }
                        }
                        Err(e) => StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(e.to_string()),
                        },
                    }
                })
            })));
        }

        if path.is_empty() {
            // 返回 home 的基本信息
            return Ok(InvokeStream::single(json!({
                "success": true,
                "data": {
                    "plugins": ["work", "agent", "setting"]
                }
            })));
        }

        if path == "config" {
            let action = input.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get");

            let config = Arc::clone(&self.config);
            let _work = Arc::clone(&self.work);
            let _agent = Arc::clone(&self.agent);
            let _setting = Arc::clone(&self.setting);
            let home_self = Arc::new(self.clone());

            return Ok(InvokeStream::Single(tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    match action {
                        "get" => {
                            let cfg = config.read().await;
                            StreamChunk {
                                data: json!({
                                    "success": true,
                                    "config": cfg.plugins
                                }),
                                done: true,
                                error: None,
                            }
                        }
                        "set" => {
                            if let Some(new_config) = input.get("config") {
                                let mut cfg = config.write().await;
                                if let Some(obj) = new_config.as_object() {
                                    for (k, v) in obj {
                                        cfg.plugins.insert(k.clone(), v.clone());
                                    }
                                }
                                // 分发配置到子插件
                                drop(cfg);
                                let cfg = config.read().await;
                                home_self.distribute_configs(&cfg.plugins).await;
                            }
                            StreamChunk {
                                data: json!({ "success": true }),
                                done: true,
                                error: None,
                            }
                        }
                        "save" => {
                            // 先收集所有子插件配置
                            let collected = home_self.collect_plugin_configs().await;
                            {
                                let mut cfg = config.write().await;
                                cfg.plugins = collected;
                            }
                            match home_self.save_config().await {
                                Ok(()) => StreamChunk {
                                    data: json!({ "success": true, "message": "配置已保存" }),
                                    done: true,
                                    error: None,
                                },
                                Err(e) => StreamChunk {
                                    data: json!({}),
                                    done: true,
                                    error: Some(e.to_string()),
                                },
                            }
                        }
                        "load" => {
                            match home_self.load_config().await {
                                Ok(()) => {
                                    let cfg = config.read().await;
                                    home_self.distribute_configs(&cfg.plugins).await;
                                    StreamChunk {
                                        data: json!({ "success": true, "config": cfg.plugins }),
                                        done: true,
                                        error: None,
                                    }
                                }
                                Err(e) => StreamChunk {
                                    data: json!({}),
                                    done: true,
                                    error: Some(e.to_string()),
                                },
                            }
                        }
                        "collect" => {
                            let collected = home_self.collect_plugin_configs().await;
                            StreamChunk {
                                data: json!({
                                    "success": true,
                                    "config": collected
                                }),
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

        let (plugin, sub_path) = self.route(path)?;
        plugin.invoke(&sub_path, input)
    }
}

// 实现 Clone（需要手动实现因为包含 dyn Plugin）
impl Clone for HomePlugin {
    fn clone(&self) -> Self {
        Self {
            meta: self.meta.clone(),
            work: Arc::clone(&self.work),
            agent: Arc::clone(&self.agent),
            setting: Arc::clone(&self.setting),
            config: Arc::clone(&self.config),
        }
    }
}
