//! HTTP 请求工具 - 实现 Tool trait

use crate::symbio_core::{
    Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest, PluginInvokeResponse,
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

    async fn execute_inner(&self, args: &Value) -> PluginInvokeResponse<Value> {
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

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let data = self.execute_inner(&args).await?;
        Ok(serde_json::to_value(&data)?)
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

/// 检查是否为私有 / 本地 / 不可路由的主机。
///
/// 判据走 **IP 语义**而不是字符串前缀：`127.0.0.1` 是前缀比较唯一认得出的回环地址，
/// 但 `127.0.0.2`、`0.0.0.0`、`::1`、`::ffff:127.0.0.1` 全都不是它，前缀比较会逐个
/// 放行——而这些地址指向的正是本机与内网（`169.254.169.254` 是云元数据端点）。
fn is_private_or_local_host(host: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local() // fc00::/7
                    || v6.is_unicast_link_local() // fe80::/10
                    // IPv4-mapped（`::ffff:127.0.0.1`）：按内层 IPv4 判
                    || v6.to_ipv4_mapped()
                        .is_some_and(|v4| v4.is_loopback() || v4.is_private() || v4.is_link_local())
            }
        },
        // 不是 IP：内网域名的常见后缀
        Err(_) => host.ends_with(".local") || host.ends_with(".internal"),
    }
}

/// 检查主机是否匹配白名单（按**域名标签边界**比较，大小写不敏感）。
///
/// 通配项 `*.example.com` 必须等价于「`example.com` 本身 或 以 `.example.com` 结尾」。
/// 早先用裸 `ends_with("example.com")`，会把 `evil-example.com` 一并放行——与项目在
/// VDFS 地址规则里记过的教训同源：**按字符串前后缀代替按段比较**。
///
/// 原实现另有一处笔误 `host == &suffix[1..]`：`"example.com"[1..]` 是 `"xample.com"`，
/// 本意应是「裸域名自身」，而这一点已由 `ends_with(".suffix")` 的反面覆盖，故删掉。
fn host_matches_allowlist(host: &str, allowlist: &[String]) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    allowlist.iter().any(|allowed| {
        let allowed = allowed.trim().to_ascii_lowercase();
        if allowed == "*" {
            return true;
        }
        match allowed.strip_prefix("*.") {
            Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
            None => host == allowed,
        }
    })
}

#[cfg(test)]
#[path = "http_request.test.rs"]
mod tests;
