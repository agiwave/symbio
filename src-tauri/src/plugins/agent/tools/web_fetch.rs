//! Web 获取工具

use crate::core::types::PluginError;
use serde_json::{json, Value};

const MAX_RESPONSE_SIZE: usize = 1_048_576; // 1MB
const REQUEST_TIMEOUT_SECS: u64 = 30;

pub struct WebFetchTool;

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute(&self, args: Value) -> Result<Value, PluginError> {
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
