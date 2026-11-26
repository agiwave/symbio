//! HTTP 请求工具 —— 工具端口执行体（`PortExecutor`）

use super::net_guard::{
    agent_network_allowed, extract_host, read_response_text, send_with_safe_redirects,
    validate_http_url, OutboundBody, OutboundRequest, SandboxPolicy,
};
use super::web_config::WebConfig;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use symbio_plugin_api::{
    missing_caller, port_result, sandbox_of, CapabilityMeta, ExecEnv, PluginError,
    PluginInvokeRequest, PluginInvokeResponse, PortExecContext, PortExecutor, PortInput,
    PortOutcome, SandboxFace,
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
        sandbox: Option<&dyn SandboxFace>,
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

        // Agent 声明的出站白名单（第三道闸）。放在宿主白名单**之后**：
        // 两个清单取交集——宿主说「这些站能去」、Agent 说「这些站我要去」，
        // 少任何一票都不发。发送循环还会对每一个重定向跳重复这道判据。
        agent_network_allowed(&extract_host(validated_url.as_str())?, agent_scope)?;

        // 验证方法
        let http_method = self.validate_method(method)?;

        // 解析请求头
        let headers = args
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|headers| {
                headers
                    .iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // 解析请求体；保存为可重放模板，307/308 需要原样带到下一跳。
        let body = args.get("body").map(|body| {
            if let Some(body) = body.as_str() {
                OutboundBody::Text(body.to_string())
            } else {
                OutboundBody::Json(body.clone())
            }
        });
        let request = OutboundRequest {
            method: http_method,
            headers,
            body,
        };
        let timeout = Duration::from_secs(timeout_secs);

        // 超时包住整个手动重定向链与响应体读取，避免逐跳各给一份 timeout
        // 后把实际总时限放大到 6 倍。reqwest 侧仍保留单请求超时作为内层保护。
        let result = tokio::time::timeout(timeout, async {
            let response = send_with_safe_redirects(
                validated_url,
                request,
                timeout,
                None,
                SandboxPolicy::new(
                    sandbox,
                    "http_request",
                    args,
                    symbio_plugin_api::CapabilityRiskLevel::High,
                ),
                None,
                |host| {
                    if !host_matches_allowlist(host, &allowed_domains) {
                        return Err(PluginError::ValidationError(format!(
                            "主机 '{host}' 不在 allowed_domains 中"
                        )));
                    }
                    agent_network_allowed(host, agent_scope)
                },
            )
            .await?;

            let status = response.status();
            let headers_out: Vec<(String, String)> = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            let (body_text, truncated, size) =
                read_response_text(response, self.max_response_size).await?;

            Ok::<_, PluginError>(json!({
                "success": true,
                "status": status.as_u16(),
                "status_text": status.canonical_reason().unwrap_or(""),
                "headers": headers_out,
                "body": body_text,
                "truncated": truncated,
                "size": size
            }))
        })
        .await;

        match result {
            Ok(result) => result,
            Err(_) => Err(PluginError::InternalError(format!(
                "请求超时：{timeout_secs}s 内未完成（可调大 web_timeout 后重试）",
            ))),
        }
    }

    fn validate_url(url: &str, allowed_domains: &[String]) -> Result<url::Url, PluginError> {
        let parsed = validate_http_url(url)?;
        let host = extract_host(parsed.as_str())?;

        // 检查允许的域名
        if !host_matches_allowlist(&host, allowed_domains) {
            return Err(PluginError::ValidationError(format!(
                "主机 '{host}' 不在 allowed_domains 中"
            )));
        }

        Ok(parsed)
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

impl HttpRequestTool {
    pub fn meta(&self) -> CapabilityMeta {
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

    async fn run(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let scope = symbio_plugin_api::agent_perms_of(ctx.as_ref());
        let sandbox = sandbox_of(ctx.as_ref());
        let data = self
            .execute_inner(&args, scope.as_ref(), sandbox.as_deref())
            .await?;
        Ok(serde_json::to_value(&data)?)
    }
}

#[async_trait]
impl PortExecutor for HttpRequestTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return missing_caller();
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
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
