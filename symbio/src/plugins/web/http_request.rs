//! HTTP 请求工具 - 实现 Tool trait

use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_RESPONSE_SIZE: usize = 1_048_576; // 1 MB

/// HTTP 请求工具
#[derive(Clone)]
pub struct HttpRequestTool {
    allowed_domains: Vec<String>,
    max_response_size: usize,
    timeout_secs: u64,
}

impl HttpRequestTool {
    pub fn new() -> Self {
        Self {
            allowed_domains: vec!["*".to_string()],
            max_response_size: DEFAULT_MAX_RESPONSE_SIZE,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    async fn execute_inner(&self, args: &Value) -> InvokeResponse<Value> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'url' 参数".to_string()))?;

        let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");

        // 验证 URL
        let validated_url = self.validate_url(url)?;

        // 验证方法
        let http_method = self.validate_method(method)?;

        // 解析请求头
        let headers = args.get("headers").and_then(|v| v.as_object());

        // 解析请求体
        let body = args.get("body");

        // 构建客户端
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .build()
            .map_err(|e| PluginError::InternalError(format!("创建 HTTP 客户端失败: {e}")))?;

        // 构建请求
        let mut request = match http_method {
            reqwest::Method::GET => client.get(&validated_url),
            reqwest::Method::POST => client.post(&validated_url),
            reqwest::Method::PUT => client.put(&validated_url),
            reqwest::Method::DELETE => client.delete(&validated_url),
            reqwest::Method::PATCH => client.patch(&validated_url),
            reqwest::Method::HEAD => client.head(&validated_url),
            reqwest::Method::OPTIONS => client.request(reqwest::Method::OPTIONS, &validated_url),
            _ => {
                return Err(PluginError::ValidationError(format!(
                    "不支持的 HTTP 方法: {method}"
                )))
            }
        };

        // 添加请求头
        if let Some(headers_obj) = headers {
            for (key, value) in headers_obj {
                if let Some(str_val) = value.as_str() {
                    request = request.header(key, str_val);
                }
            }
        }

        // 添加请求体
        if let Some(body_val) = body {
            if let Some(body_str) = body_val.as_str() {
                request = request.body(body_str.to_string());
            } else {
                request = request.json(body_val);
            }
        }

        // 发送请求
        let response = request
            .send()
            .await
            .map_err(|e| PluginError::InternalError(format!("请求失败: {e}")))?;

        let status = response.status();
        let headers_out: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        // 读取响应体
        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| PluginError::InternalError(format!("读取响应失败: {e}")))?;

        // 检查响应大小
        let truncated = body_bytes.len() > self.max_response_size;
        let body_text = if truncated {
            String::from_utf8_lossy(&body_bytes[..self.max_response_size]).to_string()
        } else {
            String::from_utf8_lossy(&body_bytes).to_string()
        };

        Ok(json!({
            "success": true,
            "status": status.as_u16(),
            "status_text": status.canonical_reason().unwrap_or(""),
            "headers": headers_out,
            "body": body_text,
            "truncated": truncated,
            "size": body_bytes.len()
        }))
    }

    fn validate_url(&self, url: &str) -> Result<String, PluginError> {
        let url = url.trim();

        if url.is_empty() {
            return Err(PluginError::ValidationError("URL 不能为空".to_string()));
        }

        if url.chars().any(char::is_whitespace) {
            return Err(PluginError::ValidationError("URL 不能包含空格".to_string()));
        }

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(PluginError::ValidationError(
                "只允许 http:// 和 https:// URL".to_string(),
            ));
        }

        // 提取主机
        let host = extract_host(url)?;

        // 阻止私有/本地主机（SSRF 防护）
        if is_private_or_local_host(&host) {
            return Err(PluginError::ValidationError(format!(
                "阻止本地/私有主机: {host}"
            )));
        }

        // 检查允许的域名
        if !host_matches_allowlist(&host, &self.allowed_domains) {
            return Err(PluginError::ValidationError(format!(
                "主机 '{host}' 不在 allowed_domains 中"
            )));
        }

        Ok(url.to_string())
    }

    fn validate_method(&self, method: &str) -> Result<reqwest::Method, PluginError> {
        match method.to_uppercase().as_str() {
            "GET" => Ok(reqwest::Method::GET),
            "POST" => Ok(reqwest::Method::POST),
            "PUT" => Ok(reqwest::Method::PUT),
            "DELETE" => Ok(reqwest::Method::DELETE),
            "PATCH" => Ok(reqwest::Method::PATCH),
            "HEAD" => Ok(reqwest::Method::HEAD),
            "OPTIONS" => Ok(reqwest::Method::OPTIONS),
            _ => Err(PluginError::ValidationError(format!(
                "不支持的 HTTP 方法: {method}。支持: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS"
            ))),
        }
    }
}

#[async_trait]
impl Capability for HttpRequestTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "http_request".to_string(),
            description: "发送 HTTP 请求。支持 GET、POST、PUT、DELETE 等方法。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "请求 URL（仅支持 HTTP/HTTPS）"
                    },
                    "method": {
                        "type": "string",
                        "enum": ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"],
                        "description": "HTTP 方法（默认 GET）"
                    },
                    "headers": {
                        "type": "object",
                        "description": "请求头"
                    },
                    "body": {
                        "description": "请求体（字符串或 JSON 对象）"
                    }
                },
                "required": ["url"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::SystemOperation),
            examples: Some(vec![
                "url='https://api.example.com/data'".to_string(),
                "url='https://api.example.com/users', method='POST', body={'name':'test'}"
                    .to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
        let data = self.execute_inner(&args).await?;
        Ok(PluginPayload::new(&data))
    }
}

impl Default for HttpRequestTool {
    fn default() -> Self {
        Self::new()
    }
}

/// 从 URL 提取主机
fn extract_host(url: &str) -> Result<String, PluginError> {
    let url = url::Url::parse(url)
        .map_err(|e| PluginError::ValidationError(format!("无效的 URL: {e}")))?;

    url.host_str()
        .map(|h| h.to_string())
        .ok_or_else(|| PluginError::ValidationError("URL 缺少主机".to_string()))
}

/// 检查是否为私有或本地主机
fn is_private_or_local_host(host: &str) -> bool {
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }

    // 私有 IP 范围
    if host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("172.16.")
        || host.starts_with("172.17.")
        || host.starts_with("172.18.")
        || host.starts_with("172.19.")
        || host.starts_with("172.20.")
        || host.starts_with("172.21.")
        || host.starts_with("172.22.")
        || host.starts_with("172.23.")
        || host.starts_with("172.24.")
        || host.starts_with("172.25.")
        || host.starts_with("172.26.")
        || host.starts_with("172.27.")
        || host.starts_with("172.28.")
        || host.starts_with("172.29.")
        || host.starts_with("172.30.")
        || host.starts_with("172.31.")
        || host.starts_with("169.254.")
    {
        return true;
    }

    // 本地域名
    if host.ends_with(".local") || host.ends_with(".localhost") || host.ends_with(".internal") {
        return true;
    }

    false
}

/// 检查主机是否匹配白名单
fn host_matches_allowlist(host: &str, allowlist: &[String]) -> bool {
    for allowed in allowlist {
        if allowed == "*" {
            return true;
        }
        if let Some(suffix) = allowed.strip_prefix("*.") {
            if host.ends_with(suffix) || host == &suffix[1..] {
                return true;
            }
        }
        if host == allowed {
            return true;
        }
    }
    false
}
