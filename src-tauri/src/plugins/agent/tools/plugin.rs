//! Tools 插件实现
//!
//! 提供文件操作、Shell 命令、Web 访问等工具

use super::policy::SecurityPolicy;
use super::{file_read::FileReadTool, file_write::FileWriteTool, shell::ShellTool, web_fetch::WebFetchTool};
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Tools 插件
pub struct ToolsPlugin {
    meta: PluginMeta,
    security: Arc<RwLock<SecurityPolicy>>,
}

impl ToolsPlugin {
    pub fn new(workspace_dir: std::path::PathBuf) -> Self {
        let security = SecurityPolicy::new(workspace_dir);
        Self {
            meta: PluginMeta {
                name: "tools".to_string(),
                description: "文件操作和 Shell 命令工具".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "tool": {
                            "type": "string",
                            "enum": ["read_file", "write_file", "shell", "web_fetch", "list"],
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
            },
            security: Arc::new(RwLock::new(security)),
        }
    }
}

impl Default for ToolsPlugin {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_default())
    }
}

#[async_trait]
impl Plugin for ToolsPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let tool = input.get("tool")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 tool 参数".to_string()))?
            .to_string();

        let params = input.get("params").cloned().unwrap_or(json!({}));
        let security = Arc::clone(&self.security);

        let stream = async_stream::stream! {
            match tool.as_str() {
                "list" => {
                    yield StreamChunk {
                        data: json!({
                            "tools": [
                                {"name": "read_file", "description": "读取文件内容"},
                                {"name": "write_file", "description": "写入文件内容"},
                                {"name": "shell", "description": "执行 Shell 命令"},
                                {"name": "web_fetch", "description": "获取网页内容"},
                            ]
                        }),
                        done: true,
                        error: None,
                    };
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
