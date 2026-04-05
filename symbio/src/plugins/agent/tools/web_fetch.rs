//! Web 获取工具 - 实现 Plugin trait

use crate::symbio_core::traits::Plugin;
use crate::symbio_core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{json, Value};

const MAX_RESPONSE_SIZE: usize = 1_048_576; // 1MB
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Web 获取工具
pub struct WebFetchTool;

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }

    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "web_fetch".to_string(),
            description: "获取网页内容。支持 HTTP/HTTPS 协议。".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "网页 URL"
                    }
                },
                "required": ["url"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "status": { "type": "integer" },
                    "content_type": { "type": "string" },
                    "content": { "type": "string" },
                    "truncated": { "type": "boolean" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    async fn execute_inner(&self, args: Value) -> Result<Value, PluginError> {
        let url = args.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 url 参数".to_string()))?;

        // 验证 URL
        let parsed = url::Url::parse(url)
            .map_err(|e| PluginError::ValidationError(format!("无效的 URL: {}", e)))?;

        // 只允许 http/https
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(PluginError::ValidationError("只支持 HTTP/HTTPS 协议".into()));
        }

        // 创建客户端
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent("Symbio/0.1.0")
            .build()
            .map_err(|e| PluginError::InternalError(format!("创建 HTTP 客户端失败: {}", e)))?;

        // 发送请求
        let response = client.get(url)
            .send()
            .await
            .map_err(|e| PluginError::InternalError(format!("请求失败: {}", e)))?;

        let status = response.status();
        let content_type = response.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        // 读取响应体
        let body = response.text()
            .await
            .map_err(|e| PluginError::InternalError(format!("读取响应失败: {}", e)))?;

        // 截断内容
        let (content, truncated) = if body.len() > MAX_RESPONSE_SIZE {
            (body[..MAX_RESPONSE_SIZE].to_string(), true)
        } else {
            (body, false)
        };

        Ok(json!({
            "success": true,
            "status": status.as_u16(),
            "content_type": content_type,
            "content": content,
            "truncated": truncated,
            "url": url
        }))
    }
}

#[async_trait]
impl Plugin for WebFetchTool {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(Self::create_meta())
        } else {
            Err(PluginError::NotFound(format!("路径不存在: {}", path)))
        }
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if !path.is_empty() {
            return Err(PluginError::NotFound(format!("路径不存在: {}", path)));
        }

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match self.execute_inner(input).await {
                    Ok(result) => StreamChunk {
                        data: result,
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
        });

        Ok(InvokeStream::Single(result))
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}