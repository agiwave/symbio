//! Work 插件 - 工作区路径管理
//!
//! 只提供工作区的打开和获取核心接口

use crate::symbio_core::traits::Plugin;
use crate::symbio_core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, Weak};

/// Work 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkConfig {
    /// 工作区路径
    #[serde(default = "default_workspace_path")]
    pub workspace_path: String,
    /// 最近打开的工作区列表
    #[serde(default)]
    pub recent_workspaces: Vec<String>,
}

fn default_workspace_path() -> String { 
    dirs::home_dir()
        .map(|p| p.join("projects").to_string_lossy().to_string())
        .unwrap_or_else(|| "~/projects".to_string())
}

impl Default for WorkConfig {
    fn default() -> Self {
        Self {
            workspace_path: default_workspace_path(),
            recent_workspaces: Vec::new(),
        }
    }
}

pub struct WorkPlugin {
    meta: PluginMeta,
    config: Arc<Mutex<WorkConfig>>,
    /// 父插件引用（用于保存配置）
    parent: Option<Weak<dyn Plugin>>,
}

impl WorkPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "work".to_string(),
            description: "工作区路径管理插件".to_string(),
            version: "0.2.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_workspace", "set_workspace"],
                        "description": "操作类型"
                    },
                    "path": { "type": "string", "description": "工作区路径（set_workspace 时使用）" }
                }
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "workspace_path": { "type": "string" },
                    "expanded_path": { "type": "string" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        let config = Arc::new(Mutex::new(WorkConfig::default()));

        // 初始化时尝试切换到默认工作区目录
        let default_path = default_workspace_path();
        let expanded = shellexpand::tilde(&default_path).to_string();
        if let Err(e) = std::env::set_current_dir(&expanded) {
            eprintln!("[work] 初始化切换目录失败 ({}): {}", default_path, e);
        } else {
            eprintln!("[work] 初始化已切换到目录: {}", expanded);
        }

        WorkPlugin {
            meta: Self::create_meta(),
            config,
            parent,
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// 获取配置 Schema
    fn config_schema() -> Value {
        json!({
            "workspace_path": {
                "type": "string",
                "title": "工作区路径",
                "description": "当前工作区目录",
                "default": default_workspace_path()
            },
            "recent_workspaces": {
                "type": "array",
                "title": "最近工作区",
                "description": "最近打开的工作区列表",
                "items": { "type": "string" }
            }
        })
    }

    /// 获取工作区路径
    fn get_workspace(&self) -> Value {
        let cfg = self.config.lock().unwrap();
        let path = shellexpand::tilde(&cfg.workspace_path).to_string();
        json!({
            "success": true,
            "workspace_path": cfg.workspace_path,
            "expanded_path": path
        })
    }

    /// 设置工作区路径
    fn set_workspace(&self, path: &str) -> Value {
        let mut cfg = self.config.lock().unwrap();

        // 更新最近工作区列表
        cfg.recent_workspaces.retain(|p| p != path);
        cfg.recent_workspaces.insert(0, path.to_string());
        cfg.recent_workspaces.truncate(10); // 保留最近 10 个

        cfg.workspace_path = path.to_string();

        // 切换当前工作目录到工作区目录
        let expanded_path = shellexpand::tilde(path).to_string();
        if let Err(e) = std::env::set_current_dir(&expanded_path) {
            eprintln!("[work] 切换当前目录失败: {}", e);
        } else {
            eprintln!("[work] 已切换当前目录到: {}", expanded_path);
        }

        // 通知父插件保存配置
        if let Some(p) = self.get_parent() {
            let _ = p.invoke("save_config", json!({}));
        }

        json!({
            "success": true,
            "workspace_path": path
        })
    }
}

impl Default for WorkPlugin {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait::async_trait]
impl Plugin for WorkPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Work 配置管理".to_string(),
                version: "0.2.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["get", "set", "schema"],
                            "description": "操作类型"
                        },
                        "config": {
                            "type": "object",
                            "description": "配置数据（set 操作时使用）"
                        }
                    },
                    "required": ["action"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "config": { "type": "object" },
                        "schema": { "type": "object" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            });
        }
        Ok(self.meta.clone())
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 处理 config path
        if path == "config" {
            let action = input.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get");
            
            let config = Arc::clone(&self.config);
            let parent = self.get_parent();

            let result = match action {
                "get" => {
                    let cfg = config.lock().unwrap();
                    json!({
                        "workspace_path": cfg.workspace_path,
                        "recent_workspaces": cfg.recent_workspaces
                    })
                }
                "set" => {
                    if let Some(new_config) = input.get("config") {
                        let mut cfg = config.lock().unwrap();
                        if let Some(v) = new_config.get("workspace_path").and_then(|v| v.as_str()) {
                            cfg.workspace_path = v.to_string();
                        }
                        if let Some(v) = new_config.get("recent_workspaces").and_then(|v| v.as_array()) {
                            cfg.recent_workspaces = v.iter()
                                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                .collect();
                        }
                    }
                    // 通知父插件保存配置
                    if let Some(p) = parent {
                        let _ = p.invoke("save_config", json!({}));
                    }
                    json!({ "success": true })
                }
                "schema" => {
                    json!({
                        "success": true,
                        "schema": Self::config_schema()
                    })
                }
                _ => json!({
                    "error": format!("未知操作: {}", action)
                }),
            };

            return Ok(InvokeStream::single(result));
        }

        // 兼容旧的 workspace_path action
        let action = input.get("action").and_then(|v| v.as_str());

        // 处理直接路径调用（如 work/workspace_path）
        if path == "workspace_path" || action == Some("workspace_path") {
            return Ok(InvokeStream::single(self.get_workspace()));
        }

        let result = match action {
            Some("get_workspace") | Some("workspace_path") | None => {
                // 默认行为或明确请求时返回工作区路径
                self.get_workspace()
            }
            Some("set_workspace") => {
                let new_path = input.get("path").and_then(|v| v.as_str())
                    .or_else(|| input.get("workspace_path").and_then(|v| v.as_str()));

                if let Some(path) = new_path {
                    self.set_workspace(path)
                } else {
                    return Err(PluginError::ValidationError("缺少 path 参数".to_string()));
                }
            }
            Some(other) => {
                return Err(PluginError::ValidationError(format!("未知操作: {}。work 插件仅支持 get_workspace 和 set_workspace 操作。", other)));
            }
        };

        Ok(InvokeStream::single(result))
    }
}
