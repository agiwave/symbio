//! Tools 插件实现
//!
//! 提供文件操作、Shell 命令、Web 访问等工具
//! 每个工具都是独立的 Plugin 实例

use super::policy::SecurityPolicy;
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
use crate::core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
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

/// Tools 插件 - 持有所有工具实例
pub struct ToolsPlugin {
    meta: PluginMeta,
    config: Arc<RwLock<ToolsConfig>>,
    /// 子工具实例
    tools: HashMap<String, Arc<dyn Plugin>>,
    /// 父插件引用（用于保存配置）
    parent: Option<Weak<dyn Plugin>>,
    /// 安全策略（用于动态更新工作区路径）
    security: Arc<SecurityPolicy>,
}

impl ToolsPlugin {
    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: ToolsConfig) -> Self {
        // 默认使用当前目录，实际工作区通过 get_workspace_path() 获取
        let default_workspace = std::env::current_dir().unwrap_or_default();
        let security = Arc::new(SecurityPolicy::new(default_workspace.clone()));

        // 创建所有工具实例（都使用同一个 security 引用）
        let tools: HashMap<String, Arc<dyn Plugin>> = vec![
            ("read_file", Arc::new(FileReadTool::new(Arc::clone(&security))) as Arc<dyn Plugin>),
            ("write_file", Arc::new(FileWriteTool::new(Arc::clone(&security))) as Arc<dyn Plugin>),
            ("file_edit", Arc::new(FileEditTool::new(Arc::clone(&security))) as Arc<dyn Plugin>),
            ("shell", Arc::new(ShellTool::new(Arc::clone(&security))) as Arc<dyn Plugin>),
            ("web_fetch", Arc::new(WebFetchTool::new()) as Arc<dyn Plugin>),
            ("web_search", Arc::new(WebSearchTool::new()) as Arc<dyn Plugin>),
            ("glob_search", Arc::new(GlobSearchTool::new(Arc::clone(&security))) as Arc<dyn Plugin>),
            ("content_search", Arc::new(ContentSearchTool::new(Arc::clone(&security))) as Arc<dyn Plugin>),
            ("http_request", Arc::new(HttpRequestTool::new()) as Arc<dyn Plugin>),
        ].into_iter().map(|(k, v)| (k.to_string(), v)).collect();

        Self {
            meta: PluginMeta {
                name: "tools".to_string(),
                description: "文件操作和 Shell 命令工具集".to_string(),
                version: "0.1.0".to_string(),
                input: None,
                output: None,
                author: Some("Symbio Team".to_string()),
            },
            config: Arc::new(RwLock::new(config)),
            tools,
            parent,
            security,
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }
    
    /// 获取工作区路径（通过绝对路径 /work/workspace_path 从 root 获取）
    fn get_workspace_path(&self) -> std::path::PathBuf {
        if let Some(parent) = self.get_parent() {
            // 使用绝对路径 /work/workspace_path 获取工作区
            match parent.invoke("/work/workspace_path", json!({})) {
                Ok(InvokeStream::Single(chunk)) if chunk.error.is_none() => {
                    if let Some(path) = chunk.data.get("expanded_path").and_then(|v| v.as_str()) {
                        eprintln!("[tools] got workspace path from /work/workspace_path: {}", path);
                        return std::path::PathBuf::from(path);
                    }
                    if let Some(path) = chunk.data.get("workspace_path").and_then(|v| v.as_str()) {
                        let expanded = shellexpand::tilde(path).to_string();
                        eprintln!("[tools] got workspace path: {} -> {}", path, expanded);
                        return std::path::PathBuf::from(expanded);
                    }
                }
                Ok(InvokeStream::Single(chunk)) => {
                    eprintln!("[tools] failed to get workspace path: {:?}", chunk.error);
                }
                Err(e) => {
                    eprintln!("[tools] error getting workspace path: {:?}", e);
                }
                _ => {}
            }
        }
        // 回退到当前目录
        std::env::current_dir().unwrap_or_default()
    }

    /// 获取所有工具的 OpenAI 格式定义
    fn get_all_tools_openai_format(&self) -> Vec<Value> {
        self.tools.iter().map(|(name, tool)| {
            let meta = tool.meta("").unwrap_or_else(|_| PluginMeta {
                name: name.clone(),
                description: "".to_string(),
                version: "0.1.0".to_string(),
                input: None,
                output: None,
                author: None,
            });
            
            json!({
                "type": "function",
                "function": {
                    "name": format!("tools::{}", name),
                    "description": meta.description,
                    "parameters": meta.input.unwrap_or(json!({}))
                }
            })
        }).collect()
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
        // 空路径返回插件本身的 meta
        if path.is_empty() {
            return Ok(self.meta.clone());
        }

        // config path
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

        // _list path - 返回所有工具列表
        if path == "_list" {
            return Ok(PluginMeta {
                name: "_list".to_string(),
                description: "列出所有可用工具".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {}
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "tools": { "type": "array" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            });
        }

        // _search path - 搜索工具
        if path == "_search" {
            return Ok(PluginMeta {
                name: "_search".to_string(),
                description: "搜索工具".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "keywords": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "搜索关键词"
                        }
                    }
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "tools": { "type": "array" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            });
        }

        // 子工具路径 - 路由到对应工具
        if let Some(tool) = self.tools.get(path) {
            return tool.meta("");
        }

        Err(PluginError::NotFound(format!("路径不存在: {}", path)))
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 绝对路径（以 / 开头）：转发给父插件处理
        if path.starts_with('/') {
            if let Some(parent) = self.get_parent() {
                eprintln!("[tools] forwarding absolute path '{}' to parent", path);
                return parent.invoke(path, input);
            } else {
                return Err(PluginError::NotFound(format!("无法解析绝对路径 '{}'：没有父插件", path)));
            }
        }
        
        // config path
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
        
        // _workspace path - 获取当前工作区路径
        if path == "_workspace" {
            let workspace = self.get_workspace_path();
            return Ok(InvokeStream::Single(StreamChunk {
                data: json!({ 
                    "success": true,
                    "workspace_path": workspace.to_string_lossy().to_string()
                }),
                done: true,
                error: None,
            }));
        }

        // _list path - 返回所有工具列表
        if path == "_list" {
            let tools = self.get_all_tools_openai_format();
            return Ok(InvokeStream::Single(StreamChunk {
                data: json!({ "tools": tools }),
                done: true,
                error: None,
            }));
        }

        // available_tools path - 返回所有工具的 meta（通用接口）
        if path == "available_tools" {
            let tools = self.available_tools();
            return Ok(InvokeStream::single(json!({
                "success": true,
                "tools": tools
            })));
        }

        // _search path - 搜索工具
        if path == "_search" {
            let keywords: Vec<String> = input.get("keywords")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_lowercase()))
                    .collect())
                .unwrap_or_default();

            let matched: Vec<Value> = self.tools.iter()
                .filter(|(name, tool)| {
                    if keywords.is_empty() {
                        return true;
                    }
                    let name_lower = name.to_lowercase();
                    if let Ok(meta) = tool.meta("") {
                        let desc_lower = meta.description.to_lowercase();
                        keywords.iter().any(|kw| 
                            name_lower.contains(kw) || desc_lower.contains(kw)
                        )
                    } else {
                        keywords.iter().any(|kw| name_lower.contains(kw))
                    }
                })
                .map(|(name, tool)| {
                    let meta = tool.meta("").unwrap_or_else(|_| PluginMeta {
                        name: name.clone(),
                        description: "".to_string(),
                        version: "0.1.0".to_string(),
                        input: None,
                        output: None,
                        author: None,
                    });
                    
                    json!({
                        "type": "function",
                        "function": {
                            "name": format!("tools::{}", name),
                            "description": meta.description,
                            "parameters": meta.input.unwrap_or(json!({}))
                        }
                    })
                })
                .collect();

            return Ok(InvokeStream::Single(StreamChunk {
                data: json!({ "tools": matched }),
                done: true,
                error: None,
            }));
        }

        // 子工具路径 - 路由到对应工具
        if let Some(tool) = self.tools.get(path) {
            // 在执行工具调用前，更新工作区路径
            let workspace = self.get_workspace_path();
            let security = Arc::clone(&self.security);
            let input_clone = input.clone();
            
            // 异步更新工作区路径
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    security.update_workspace_dir(workspace).await;
                })
            });
            
            return tool.invoke("", input_clone);
        }

        // 兼容旧的 tool/params 格式
        if path.is_empty() {
            let tool_name = input.get("tool")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PluginError::ValidationError("缺少 tool 参数".to_string()))?;

            let params = input.get("params").cloned().unwrap_or(json!({}));

            if let Some(tool) = self.tools.get(tool_name) {
                // 在执行工具调用前，更新工作区路径
                let workspace = self.get_workspace_path();
                let security = Arc::clone(&self.security);
                let params_clone = params.clone();
                
                // 异步更新工作区路径
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        security.update_workspace_dir(workspace).await;
                    })
                });
                
                return tool.invoke("", params_clone);
            }

            return Err(PluginError::NotFound(format!("工具不存在: {}", tool_name)));
        }

        Err(PluginError::NotFound(format!("路径不存在: {}", path)))
    }

    fn available_tools(&self) -> Vec<PluginMeta> {
        self.tools.iter().filter_map(|(_name, tool)| {
            // 获取工具的 meta，如果失败则跳过
            tool.meta("").ok()
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tools_plugin_available_tools() {
        // 创建 ToolsPlugin 实例
        let tools_plugin = ToolsPlugin::new(None, ToolsConfig::default());

        // 调用 available_tools 方法
        let tools = tools_plugin.available_tools();

        // 验证返回的工具列表不为空
        assert!(!tools.is_empty(), "ToolsPlugin should return non-empty tools list");

        // 打印所有工具名称
        println!("ToolsPlugin available tools ({} total):", tools.len());
        for tool in &tools {
            println!("  - {}", tool.name);
        }

        // 验证包含预期的工具
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        
        // 验证核心工具存在（子插件返回原始名称，由父插件添加前缀）
        assert!(tool_names.contains(&"read_file"), "Should have read_file tool");
        assert!(tool_names.contains(&"write_file"), "Should have write_file tool");
        assert!(tool_names.contains(&"shell"), "Should have shell tool");
        assert!(tool_names.contains(&"web_search"), "Should have web_search tool");
        assert!(tool_names.contains(&"glob_search"), "Should have glob_search tool");
        assert!(tool_names.contains(&"content_search"), "Should have content_search tool");
    }
}
