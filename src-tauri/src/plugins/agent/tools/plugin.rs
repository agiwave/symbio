//! Tools 插件实现
//!
//! 提供文件操作、Shell 命令、Web 访问等工具

use super::policy::SecurityPolicy;
use super::aggregation_tools::AggregationTools;
use super::{
    file_read::FileReadTool, 
    file_write::FileWriteTool, 
    file_edit::FileEditTool,
    shell::ShellTool, 
    web_fetch::WebFetchTool,
    web_search::WebSearchTool,
    glob_search::GlobSearchTool,
    content_search::ContentSearchTool,
    http_request::HttpRequestTool,
};
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// Tools 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Shell 工具开关
    #[serde(default = "default_shell_enabled")]
    pub shell_enabled: bool,
    /// 文件工具开关
    #[serde(default = "default_file_enabled")]
    pub file_enabled: bool,
    /// Web 工具开关
    #[serde(default = "default_web_enabled")]
    pub web_enabled: bool,
    /// 允许访问的路径
    #[serde(default = "default_allowed_paths")]
    pub allowed_paths: Vec<String>,
    /// 禁止执行的命令
    #[serde(default = "default_blocked_commands")]
    pub blocked_commands: Vec<String>,
    /// Shell 超时（秒）
    #[serde(default = "default_shell_timeout")]
    pub shell_timeout: u64,
    /// Web 请求超时（秒）
    #[serde(default = "default_web_timeout")]
    pub web_timeout: u64,
}

fn default_shell_enabled() -> bool { true }
fn default_file_enabled() -> bool { true }
fn default_web_enabled() -> bool { true }
fn default_allowed_paths() -> Vec<String> { vec!["~".to_string()] }
fn default_blocked_commands() -> Vec<String> { 
    vec!["rm -rf".to_string(), "sudo".to_string(), "chmod 777".to_string()] 
}
fn default_shell_timeout() -> u64 { 60 }
fn default_web_timeout() -> u64 { 30 }

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            shell_enabled: default_shell_enabled(),
            file_enabled: default_file_enabled(),
            web_enabled: default_web_enabled(),
            allowed_paths: default_allowed_paths(),
            blocked_commands: default_blocked_commands(),
            shell_timeout: default_shell_timeout(),
            web_timeout: default_web_timeout(),
        }
    }
}

/// Tools 插件
pub struct ToolsPlugin {
    meta: PluginMeta,
    config: Arc<RwLock<ToolsConfig>>,
    security: Arc<RwLock<SecurityPolicy>>,
    /// 父插件引用（用于保存配置）
    parent: Option<Weak<dyn Plugin>>,
}

impl ToolsPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "tools".to_string(),
            description: "文件操作和 Shell 命令工具".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "tool": {
                        "type": "string",
                        "enum": [
                            "read_file", "write_file", "file_edit",
                            "shell", "web_fetch", "web_search",
                            "glob_search", "content_search", "http_request",
                            "list", "search"
                        ],
                        "description": "工具名称"
                    },
                    "params": {
                        "type": "object",
                        "description": "工具参数"
                    }
                },
                "required": ["tool"]
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: ToolsConfig) -> Self {
        let workspace_dir = std::env::current_dir().unwrap_or_default();
        let security = SecurityPolicy::new(workspace_dir);
        Self {
            meta: Self::create_meta(),
            config: Arc::new(RwLock::new(config)),
            security: Arc::new(RwLock::new(security)),
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
            "shell_enabled": {
                "type": "boolean",
                "title": "启用 Shell 工具",
                "description": "允许执行 Shell 命令",
                "default": true
            },
            "file_enabled": {
                "type": "boolean",
                "title": "启用文件工具",
                "description": "允许文件读写操作",
                "default": true
            },
            "web_enabled": {
                "type": "boolean",
                "title": "启用 Web 工具",
                "description": "允许网络请求",
                "default": true
            },
            "allowed_paths": {
                "type": "array",
                "title": "允许访问的路径",
                "description": "文件工具可访问的目录列表",
                "items": { "type": "string" },
                "default": ["~"]
            },
            "blocked_commands": {
                "type": "array",
                "title": "禁止的命令",
                "description": "Shell 工具禁止执行的命令模式",
                "items": { "type": "string" },
                "default": ["rm -rf", "sudo", "chmod 777"]
            },
            "shell_timeout": {
                "type": "integer",
                "title": "Shell 超时",
                "description": "Shell 命令执行超时时间（秒）",
                "minimum": 1,
                "maximum": 3600,
                "default": 60
            },
            "web_timeout": {
                "type": "integer",
                "title": "Web 超时",
                "description": "Web 请求超时时间（秒）",
                "minimum": 1,
                "maximum": 300,
                "default": 30
            }
        })
    }
}

impl Default for ToolsPlugin {
    fn default() -> Self {
        Self::new(None, ToolsConfig::default())
    }
}

#[async_trait]
impl Plugin for ToolsPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Tools 配置管理".to_string(),
                version: "0.1.0".to_string(),
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

            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    match action {
                        "get" => {
                            let cfg = config.read().await;
                            StreamChunk {
                                data: json!({
                                    "shell_enabled": cfg.shell_enabled,
                                    "file_enabled": cfg.file_enabled,
                                    "web_enabled": cfg.web_enabled,
                                    "allowed_paths": cfg.allowed_paths,
                                    "blocked_commands": cfg.blocked_commands,
                                    "shell_timeout": cfg.shell_timeout,
                                    "web_timeout": cfg.web_timeout
                                }),
                                done: true,
                                error: None,
                            }
                        }
                        "set" => {
                            if let Some(new_config) = input.get("config") {
                                let mut cfg = config.write().await;
                                if let Some(v) = new_config.get("shell_enabled").and_then(|v| v.as_bool()) {
                                    cfg.shell_enabled = v;
                                }
                                if let Some(v) = new_config.get("file_enabled").and_then(|v| v.as_bool()) {
                                    cfg.file_enabled = v;
                                }
                                if let Some(v) = new_config.get("web_enabled").and_then(|v| v.as_bool()) {
                                    cfg.web_enabled = v;
                                }
                                if let Some(v) = new_config.get("allowed_paths").and_then(|v| v.as_array()) {
                                    cfg.allowed_paths = v.iter()
                                        .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                        .collect();
                                }
                                if let Some(v) = new_config.get("blocked_commands").and_then(|v| v.as_array()) {
                                    cfg.blocked_commands = v.iter()
                                        .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                        .collect();
                                }
                                if let Some(v) = new_config.get("shell_timeout").and_then(|v| v.as_u64()) {
                                    cfg.shell_timeout = v;
                                }
                                if let Some(v) = new_config.get("web_timeout").and_then(|v| v.as_u64()) {
                                    cfg.web_timeout = v;
                                }
                            }
                            // 通知父插件保存配置
                            if let Some(p) = parent {
                                let _ = p.invoke("save_config", json!({}));
                            }
                            StreamChunk {
                                data: json!({ "success": true }),
                                done: true,
                                error: None,
                            }
                        }
                        "schema" => {
                            StreamChunk {
                                data: json!({
                                    "success": true,
                                    "schema": Self::config_schema()
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
            });

            return Ok(InvokeStream::Single(result));
        }

        // 原有工具调用逻辑
        let tool = input.get("tool")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 tool 参数".to_string()))?
            .to_string();

        let params = input.get("params").cloned().unwrap_or(json!({}));
        let security = Arc::clone(&self.security);
        // 在 stream 外部捕获 parent 引用
        let parent_ref = self.parent.clone();

        let stream = async_stream::stream! {
            match tool.as_str() {
                "list" => {
                    // 使用 AggregationTools 列出所有可用工具
                    let agg = AggregationTools::new(parent_ref.clone());
                    match agg.list_all_tools().await {
                        Ok(result) => {
                            yield StreamChunk {
                                data: result,
                                done: true,
                                error: None,
                            };
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "search" => {
                    // 使用 AggregationTools 搜索工具
                    let agg = AggregationTools::new(parent_ref.clone());
                    let keywords: Vec<String> = params.get("keywords")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter()
                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                            .collect())
                        .unwrap_or_default();
                    
                    match agg.search_tools(keywords).await {
                        Ok(result) => {
                            yield StreamChunk {
                                data: result,
                                done: true,
                                error: None,
                            };
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "read_file" => {
                    let guard = security.read().await;
                    let tool = FileReadTool::new(Arc::new((*guard).clone()));
                    match tool.execute(params).await {
                        Ok(result) => {
                            yield StreamChunk {
                                data: result,
                                done: true,
                                error: None,
                            };
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "write_file" => {
                    let guard = security.read().await;
                    let tool = FileWriteTool::new(Arc::new((*guard).clone()));
                    match tool.execute(params).await {
                        Ok(result) => {
                            yield StreamChunk {
                                data: result,
                                done: true,
                                error: None,
                            };
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "file_edit" => {
                    let guard = security.read().await;
                    let workspace_dir = guard.workspace_dir.clone();
                    let tool = FileEditTool::new();
                    match tool.execute(&params, &workspace_dir).await {
                        Ok(result) => {
                            yield result;
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "shell" => {
                    let guard = security.read().await;
                    let tool = ShellTool::new(Arc::new((*guard).clone()));
                    match tool.execute(params).await {
                        Ok(result) => {
                            yield StreamChunk {
                                data: result,
                                done: true,
                                error: None,
                            };
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "web_fetch" => {
                    let tool = WebFetchTool::new();
                    match tool.execute(params).await {
                        Ok(result) => {
                            yield StreamChunk {
                                data: result,
                                done: true,
                                error: None,
                            };
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "web_search" => {
                    let guard = security.read().await;
                    let workspace_dir = guard.workspace_dir.clone();
                    let tool = WebSearchTool::new();
                    match tool.execute(&params, &workspace_dir).await {
                        Ok(result) => {
                            yield result;
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "glob_search" => {
                    let guard = security.read().await;
                    let workspace_dir = guard.workspace_dir.clone();
                    let tool = GlobSearchTool::new();
                    match tool.execute(&params, &workspace_dir).await {
                        Ok(result) => {
                            yield result;
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "content_search" => {
                    let guard = security.read().await;
                    let workspace_dir = guard.workspace_dir.clone();
                    let tool = ContentSearchTool::new();
                    match tool.execute(&params, &workspace_dir).await {
                        Ok(result) => {
                            yield result;
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                "http_request" => {
                    let guard = security.read().await;
                    let workspace_dir = guard.workspace_dir.clone();
                    let tool = HttpRequestTool::new();
                    match tool.execute(&params, &workspace_dir).await {
                        Ok(result) => {
                            yield result;
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(e.to_string()),
                            };
                        }
                    }
                }
                _ => {
                    yield StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("未知工具: {}", tool)),
                    };
                }
            }
        };

        Ok(InvokeStream::Stream(Box::pin(stream)))
    }
}