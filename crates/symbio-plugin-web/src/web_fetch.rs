//! Web 获取工具 —— 工具端口执行体（`PortExecutor`）

use super::net_guard::{
    agent_network_allowed, read_response_text, send_with_safe_redirects, validate_http_url,
    OutboundRequest, SandboxPolicy,
};
use super::web_config::WebConfig;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::{
    missing_caller, port_result, sandbox_of, CapabilityMeta, ExecEnv, PluginError,
    PluginInvokeRequest, PortExecContext, PortExecutor, PortInput, PortOutcome, SandboxFace,
};
use tokio::sync::RwLock;

const MAX_RESPONSE_SIZE: usize = 1_048_576; // 1MB

/// Web 获取工具
#[derive(Clone)]
pub struct WebFetchTool {
    /// 超时**逐次从配置读**——保存 `web_timeout` 后立即生效，不需要重载插件。
    config: Arc<RwLock<WebConfig>>,
}

impl WebFetchTool {
    pub fn new(config: Arc<RwLock<WebConfig>>) -> Self {
        Self { config }
    }

    async fn execute_inner(
        &self,
        args: Value,
        agent_scope: Option<&symbio_plugin_api::AgentScope>,
        sandbox: Option<&dyn SandboxFace>,
    ) -> Result<Value, PluginError> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 url 参数".to_string()))?;

        let timeout_secs = self.config.read().await.web_timeout;
        let timeout = std::time::Duration::from_secs(timeout_secs);

        // 与 http_request 共用 URL/DNS 判据；发送循环会对每个重定向跳重新校验。
        let parsed = validate_http_url(url)?;
        let request = OutboundRequest {
            method: reqwest::Method::GET,
            headers: Vec::new(),
            body: None,
        };

        // 总超时覆盖 DNS、整个重定向链和正文读取，而不是每一跳各给一份时限。
        let result = tokio::time::timeout(timeout, async {
            let response = send_with_safe_redirects(
                parsed,
                request,
                timeout,
                Some("Symbio/0.1.0"),
                SandboxPolicy::new(
                    sandbox,
                    "web_fetch",
                    &args,
                    symbio_plugin_api::CapabilityRiskLevel::Low,
                ),
                None,
                |host| agent_network_allowed(host, agent_scope),
            )
            .await?;

            let status = response.status();
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown")
                .to_string();

            let (content, truncated, _) = read_response_text(response, MAX_RESPONSE_SIZE).await?;

            // **元数据头随正文回灌**（①）：`extract_result` 的 `content` 分支
            // 只取正文 ⇒ 404 页与 200 页对模型同形、截断后半截不知情。把状态并进
            // `content` 首行——回灌文本（结果节点 `content` = 它）与 JSON 字段两条
            // 路径都拿得到，不必在 tool_executor 里为单个工具开特例。
            let content_with_meta =
                content_with_meta(status.as_u16(), &content_type, truncated, url, &content);

            Ok::<_, PluginError>(json!({
                "success": true,
                "status": status.as_u16(),
                "content_type": content_type,
                "content": content_with_meta,
                "truncated": truncated,
                "url": url
            }))
        })
        .await;

        match result {
            Ok(result) => result,
            Err(_) => Err(PluginError::InternalError(format!(
                "请求超时：{timeout_secs}s 内未收到 {url} 的响应（可调大 web_timeout 后重试）"
            ))),
        }
    }
}

impl WebFetchTool {
    pub fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "web_fetch".to_string(),
            description: "获取网页内容。支持 HTTP/HTTPS 协议。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "网页 URL"
                    }
                },
                "required": ["url"]
            }),
            category: Some(symbio_plugin_api::CapabilityCategory::Network),
            // 只读抓取，不改本地状态⇒ 低风险（与旧风险表一致）。显式声明而非依赖默认档：
            // 默认档是 Medium，那会让「读个网页」在 low 阈值会话里多要一次审批。
            risk: Some(symbio_plugin_api::CapabilityRiskLevel::Low),
            examples: Some(vec!["url='https://example.com'".to_string()]),
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
        let result = self
            .execute_inner(args, scope.as_ref(), sandbox.as_deref())
            .await?;
        Ok(serde_json::to_value(&result)?)
    }
}

#[async_trait]
impl PortExecutor for WebFetchTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return missing_caller();
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

#[cfg(test)]
#[path = "web_fetch.test.rs"]
mod tests;

/// 响应元数据头：状态码 / 内容类型 / 是否截断 / URL 并成正文首行。
///
/// 单独成函数是为了让形状可判——e2e 在无外网 CI 上拿不到真实 200/404 响应，
/// 这行格式由单测钉住（`status=404` 缺了就会让模型把错误页当正常内容）。
fn content_with_meta(
    status: u16,
    content_type: &str,
    truncated: bool,
    url: &str,
    content: &str,
) -> String {
    format!(
        "[web_fetch] status={status} · content_type={content_type} · truncated={truncated} · url={url}\n\n{content}"
    )
}
