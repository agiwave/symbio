//! Web 获取工具 - 实现 Tool trait

use super::net_guard::{extract_host, is_private_or_local_host};
use super::web_config::WebConfig;
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
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

    async fn execute_inner(&self, args: Value) -> Result<Value, PluginError> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 url 参数".to_string()))?;

        let timeout_secs = self.config.read().await.web_timeout;

        // 验证 URL
        let parsed = url::Url::parse(url)
            .map_err(|e| PluginError::ValidationError(format!("无效的 URL: {e}")))?;

        // 只允许 http/https
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(PluginError::ValidationError(
                "只支持 HTTP/HTTPS 协议".into(),
            ));
        }

        // 阻止私有/本地主机（SSRF 防护，与 `http_request` 共用同一判据）
        let host = extract_host(url)?;
        if is_private_or_local_host(&host) {
            return Err(PluginError::ValidationError(format!(
                "阻止本地/私有主机: {host}"
            )));
        }

        // 创建客户端
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .user_agent("Symbio/0.1.0")
            .build()
            .map_err(|e| PluginError::InternalError(format!("创建 HTTP 客户端失败: {e}")))?;

        // 发送请求。**错误要分流**（T-109 ②）：reqwest 对超时与连接失败的
        // `Display` 同形（都是 `error sending request…`），直接回灌则「对方慢」
        // 与「地址不通」对模型与用户同形——而两者的下一步（重试 vs 改地址）相反。
        let response = client.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                PluginError::InternalError(format!(
                    "请求超时：{timeout_secs}s 内未收到 {url} 的响应（可调大 web_timeout 后重试）"
                ))
            } else if e.is_connect() {
                PluginError::InternalError(format!(
                    "连接失败：无法连到 {url}（请检查地址与网络后重试）"
                ))
            } else {
                PluginError::InternalError(format!("请求失败: {e}"))
            }
        })?;

        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        // 读取响应体
        let body = response
            .text()
            .await
            .map_err(|e| PluginError::InternalError(format!("读取响应失败: {e}")))?;

        // 截断内容
        // 安全截断：必须落在字符边界上，否则含中文的响应体会让 `&body[..N]`
        // 直接 panic（字节索引切进多字节字符内部）。
        let (content, truncated) = if body.len() > MAX_RESPONSE_SIZE {
            let end = crate::symbio_core::text_floor_char_boundary(&body, MAX_RESPONSE_SIZE);
            (body[..end].to_string(), true)
        } else {
            (body, false)
        };

        // **元数据头随正文回灌**（T-109 ①）：`extract_result` 的 `content` 分支
        // 只取正文 ⇒ 404 页与 200 页对模型同形、截断后半截不知情。把状态并进
        // `content` 首行——回灌文本（结果节点 `content` = 它）与 JSON 字段两条
        // 路径都拿得到，不必在 tool_executor 里为单个工具开特例。
        let content_with_meta =
            content_with_meta(status.as_u16(), &content_type, truncated, url, &content);

        Ok(json!({
            "success": true,
            "status": status.as_u16(),
            "content_type": content_type,
            "content": content_with_meta,
            "truncated": truncated,
            "url": url
        }))
    }
}

#[async_trait]
impl Capability for WebFetchTool {
    fn meta(&self) -> CapabilityMeta {
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
            category: Some(crate::symbio_core::CapabilityCategory::Network),
            // 只读抓取，不改本地状态⇒ 低风险（与旧风险表一致）。显式声明而非依赖默认档：
            // 默认档是 Medium，那会让「读个网页」在 low 阈值会话里多要一次审批。
            risk: Some(crate::symbio_core::CapabilityRiskLevel::Low),
            examples: Some(vec!["url='https://example.com'".to_string()]),
            ..Default::default()
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let result = self.execute_inner(args).await?;
        Ok(serde_json::to_value(&result)?)
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
