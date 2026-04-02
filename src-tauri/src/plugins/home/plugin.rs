//! Home 插件 - 根插件，持有 work/agent/setting 子插件实例
//!
//! 简洁设计：
//! - 工厂创建时传入配置，各插件初始化完成
//! - 用户修改时调用 save_config 保存
//! - 不在运行时加载/分发配置

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
    explorer: Arc<dyn Plugin>,
    config: Arc<RwLock<GlobalConfig>>,
}

impl HomePlugin {
    pub fn new(
        work: Arc<dyn Plugin>,
        agent: Arc<dyn Plugin>,
        setting: Arc<dyn Plugin>,
        explorer: Arc<dyn Plugin>,
    ) -> Self {
        Self::new_with_config(work, agent, setting, explorer, GlobalConfig::default())
    }

    /// 带配置的构造函数（工厂使用）
    pub fn new_with_config(
        work: Arc<dyn Plugin>,
        agent: Arc<dyn Plugin>,
        setting: Arc<dyn Plugin>,
        explorer: Arc<dyn Plugin>,
        config: GlobalConfig,
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
                            "description": "子插件路径，如 work/agent/setting/explorer"
                        }
                    }
                })),
                output: None,
                author: Some("Symbio Team".to_string()),
            },
            work,
            agent,
            setting,
            explorer,
            config: Arc::new(RwLock::new(config)),
        }
    }

    /// 保存配置到文件
    async fn save_config(&self) -> Result<(), PluginError> {
        let collected = self.collect_plugin_configs().await;
        {
            let mut cfg = self.config.write().await;
            cfg.plugins = collected;
        }

        let path = config_path();
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

        Ok(())
    }

    /// 收集所有子插件配置
    async fn collect_plugin_configs(&self) -> serde_json::Map<String, Value> {
        let mut configs = serde_json::Map::new();

        if let Ok(InvokeStream::Single(chunk)) = self.work.invoke("config", json!({"action": "get"})) {
            if chunk.error.is_none() && !chunk.data.is_null() {
                configs.insert("work".to_string(), chunk.data);
            }
        }

        if let Ok(InvokeStream::Single(chunk)) = self.agent.invoke("config", json!({"action": "get"})) {
            if chunk.error.is_none() && !chunk.data.is_null() {
                if let Some(agent_config) = chunk.data.get("config") {
                    configs.insert("agent".to_string(), agent_config.clone());
                } else {
                    configs.insert("agent".to_string(), chunk.data);
                }
            }
        }

        if let Ok(InvokeStream::Single(chunk)) = self.setting.invoke("config", json!({"action": "get"})) {
            if chunk.error.is_none() && !chunk.data.is_null() {
                if let Some(setting_data) = chunk.data.get("data") {
                    configs.insert("setting".to_string(), setting_data.clone());
                } else {
                    configs.insert("setting".to_string(), chunk.data);
                }
            }
        }

        if let Ok(InvokeStream::Single(chunk)) = self.explorer.invoke("config", json!({"action": "get"})) {
            if chunk.error.is_none() && !chunk.data.is_null() {
                configs.insert("explorer".to_string(), chunk.data);
            }
        }

        configs
    }

    fn route(&self, path: &str) -> Result<(Arc<dyn Plugin>, String), PluginError> {
        let parts: Vec<&str> = path.splitn(2, '/').collect();
        let plugin_name = parts[0];
        let sub_path = parts.get(1).map(|s| s.to_string()).unwrap_or_default();

        match plugin_name {
            "work" => Ok((Arc::clone(&self.work), sub_path)),
            "agent" => Ok((Arc::clone(&self.agent), sub_path)),
            "setting" => Ok((Arc::clone(&self.setting), sub_path)),
            "explorer" => Ok((Arc::clone(&self.explorer), sub_path)),
            _ => Err(PluginError::NotFound(format!("未知的插件路径: {}", plugin_name))),
        }
    }
}

#[async_trait::async_trait]
impl Plugin for HomePlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            return Ok(self.meta.clone());
        }
        let (plugin, sub_path) = self.route(path)?;
        plugin.meta(&sub_path)
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 处理绝对路径（以 / 开头）：去掉前导 / 后路由到子插件
        let path = path.strip_prefix('/').unwrap_or(path);

        // _root - 返回 root 插件信息（用于子插件获取 root）
        if path == "_root" {
            return Ok(InvokeStream::single(json!({
                "success": true,
                "name": "home",
                "children": ["work", "agent", "setting"]
            })));
        }

        // _workspace - 快捷获取工作区路径（路由到 work/workspace_path）
        if path == "_workspace" {
            return self.work.invoke("workspace_path", input);
        }

        // 处理 /work/* 路径 - 路由到 work 插件
        if path.starts_with("/work/") {
            let sub_path = path.strip_prefix("/work/").unwrap_or_default();
            eprintln!("[home] routing /work/{} to work plugin", sub_path);
            return self.work.invoke(sub_path, input);
        }

        // save_config - 保存配置到文件
        if path == "save_config" {
            let home_self = Arc::new(self.clone());
            // 后台异步保存，不阻塞调用
            tokio::spawn(async move {
                let _ = home_self.save_config().await;
            });
            return Ok(InvokeStream::Single(StreamChunk {
                data: json!({ "success": true }),
                done: true,
                error: None,
            }));
        }

        if path.is_empty() {
            return Ok(InvokeStream::single(json!({
                "success": true,
                "data": { "plugins": ["work", "agent", "setting"] }
            })));
        }

        // 路由到子插件
        let (plugin, sub_path) = self.route(path)?;
        plugin.invoke(&sub_path, input)
    }
}

impl Clone for HomePlugin {
    fn clone(&self) -> Self {
        Self {
            meta: self.meta.clone(),
            work: Arc::clone(&self.work),
            agent: Arc::clone(&self.agent),
            setting: Arc::clone(&self.setting),
            explorer: Arc::clone(&self.explorer),
            config: Arc::clone(&self.config),
        }
    }
}