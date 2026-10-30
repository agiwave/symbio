//! HTTP 请求工具 - 实现 Tool trait

use super::net_guard::{agent_network_allowed, extract_host, is_private_or_local_host};
use super::web_config::WebConfig;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use symbio_plugin_api::{
    Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest, PluginInvokeResponse,
};
use tokio::sync::RwLock;

const DEFAULT_MAX_RESPONSE_SIZE: usize = 1_048_576; // 1 MB

/// HTTP 请求工具
#[derive(Clone)]
pub struct HttpRequestTool {
    config: Arc<RwLock<WebConfig>>,
    max_response_size: usize,
}

impl HttpRequestTool {
    /// 超时与域名白名单都**逐次从配置读**——配置保存后立即生效，不需要重载插件。
    ///
    /// 白名单**必须能从配置进来**（④）：早先在构造里写死 `vec!["*"]`，
    /// 没有任何路径能写进别的值 ⇒ `host_matches_allowlist` 恒真、「不在
    /// allowed_domains 中」的错误分支永不触发——看上去在守域名，实际什么都不守。
    pub fn new(config: Arc<RwLock<WebConfig>>) -> Self {
        Self {
            config,
            max_response_size: DEFAULT_MAX_RESPONSE_SIZE,
        }
    }

    async fn execute_inner(
        &self,
        args: &Value,
        agent_scope: Option<&symbio_plugin_api::AgentScope>,
    ) -> PluginInvokeResponse<Value> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'url' 参数".to_string()))?;

        let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");

        // 配置只在入口读一次：不在 `await` 期间持有锁（保存配置不应被在途请求阻塞）。
        let (timeout_secs, allowed_domains) = {
            let cfg = self.config.read().await;
            (cfg.web_timeout, cfg.http_allowed_domains.clone())
        };

        // 验证 URL
        let validated_url = Self::validate_url(url, &allowed_domains)?;

        // Agent 声明的出站白名单（第三道闸，D-153）。放在宿主白名单**之后**：
        // 两个清单取交集——宿主说「这些站能去」、Agent 说「这些站我要去」，
        // 少任何一票都不发。
        agent_network_allowed(&extract_host(&validated_url)?, agent_scope)?;

        // 验证方法
        let http_method = self.validate_method(method)?;

        // 解析请求头
        let headers = args.get("headers").and_then(|v| v.as_object());

        // 解析请求体
        let body = args.get("body");

        // 构建客户端
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
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

        // 发送请求。**错误要分流**（②，与 `web_fetch` 同一形状）：
        // reqwest 的 Display 把超时与连接失败收成同一句 `error sending request…`
        // ⇒ 用户看不出该重试还是该改地址。
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                PluginError::InternalError(format!(
                    "请求超时：{timeout_secs}s 内未完成（可调大 web_timeout 后重试）",
                ))
            } else if e.is_connect() {
                PluginError::InternalError(format!(
                    "连接失败：无法连到 {validated_url}（请检查地址与网络后重试）"
                ))
            } else {
                PluginError::InternalError(format!("请求失败: {e}"))
            }
        })?;

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

    fn validate_url(url: &str, allowed_domains: &[String]) -> Result<String, PluginError> {
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
        if !host_matches_allowlist(&host, allowed_domains) {
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
            category: Some(symbio_plugin_api::CapabilityCategory::SystemOperation),
            // 任意方法 + 任意 body + 任意目标主机 ⇒ 高风险。旧的风险表把`http_request`
            // 单独列为 High；改由工具自声明后，若不写这一行就会落到默认 Medium，
            // 等于悄悄放宽了审批——故显式声明。
            risk: Some(symbio_plugin_api::CapabilityRiskLevel::High),
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
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let scope = symbio_plugin_api::agent_perms_of(ctx.as_ref());
        let data = self.execute_inner(&args, scope.as_ref()).await?;
        Ok(serde_json::to_value(&data)?)
    }
}

impl Default for HttpRequestTool {
    /// 默认档取 `WebConfig` 的默认超时与默认白名单（`*`）——
    /// **唯一真源**，不在此处另写字面量（那正是「配置不生效」的成因）。
    fn default() -> Self {
        Self::new(Arc::new(RwLock::new(WebConfig::default())))
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
    // 空白名单按缺省 `*` 处理（④）：白名单是 `Vec<String>`，前端列表编辑器
    // 很容易把条目删光保存——那应当回到「不限制」，而不是静默变成「全拒」
    // （全拒等于废掉 http_request 这一整个工具，且没有任何错误指向白名单）。
    if allowlist.is_empty() {
        return true;
    }
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
