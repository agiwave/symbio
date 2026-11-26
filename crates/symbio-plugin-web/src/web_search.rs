//! Web 搜索工具 —— 工具端口执行体（`PortExecutor`）

use super::net_guard::{
    agent_network_allowed, read_response_text, send_with_safe_redirects,
    validate_upstream_http_url, OutboundBody, OutboundRequest, SandboxPolicy,
};
use super::web_config::WebConfig;
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use symbio_plugin_api::{
    missing_caller, port_result, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest,
    PluginInvokeResponse, PortExecContext, PortExecutor, PortInput, PortOutcome, SandboxFace,
};
use tokio::sync::RwLock;

const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS_LIMIT: usize = 10;
const MAX_RESPONSE_SIZE: usize = 2_097_152;

/// 三家供应商的官方上游基址与路径。**路径是供应商契约的一部分**（Tavily/Serper
/// 是 `POST /search`、DuckDuckGo HTML 是 `GET /html/?q=…`），配置只允许替换
/// 基址（⑤：此前连基址都写死 ⇒ e2e 无本地靶子可打、CI 无网即零覆盖），
/// 不允许改路径形状——那等于换了另一家服务。
const TAVILY_OFFICIAL_BASE: &str = "https://api.tavily.com";
const SERPER_OFFICIAL_BASE: &str = "https://google.serper.dev";
const DUCKDUCKGO_OFFICIAL_BASE: &str = "https://html.duckduckgo.com";

/// 取本轮回实际使用的上游基址：`search_api_base` 为空（缺省）⇒ 各家官方端点；
/// 非空 ⇒ 统一替换成该基址（剥尾随 `/`，路径由调用方拼）。
fn upstream_base(configured: &str, official: &str) -> String {
    let configured = configured.trim();
    if configured.is_empty() {
        return official.to_string();
    }
    configured.trim_end_matches('/').to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchProvider {
    Tavily,
    Serper,
    DuckDuckGo,
}

fn has_api_key(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|key| !key.trim().is_empty())
}

fn selected_provider(config: &WebConfig) -> SearchProvider {
    if has_api_key(&config.tavily_api_key) {
        SearchProvider::Tavily
    } else if has_api_key(&config.serper_api_key) {
        SearchProvider::Serper
    } else {
        SearchProvider::DuckDuckGo
    }
}

impl SearchProvider {
    fn official_base(self) -> &'static str {
        match self {
            Self::Tavily => TAVILY_OFFICIAL_BASE,
            Self::Serper => SERPER_OFFICIAL_BASE,
            Self::DuckDuckGo => DUCKDUCKGO_OFFICIAL_BASE,
        }
    }
}

/// 注册事实与实际请求共用同一 URL 投影，避免「面板显示走 A、裁决却查 B」。
/// API key 不参与 URL：Tavily 在 body，Serper 在 header；DuckDuckGo 的查询按供应商
/// 契约位于 query string。 WHY：沙箱只掌握配置与工具参数，无法从模型 query 反推出上游。
pub(super) fn upstream_url(config: &WebConfig, args: &Value) -> String {
    let provider = selected_provider(config);
    let base = upstream_base(&config.search_api_base, provider.official_base());
    match provider {
        SearchProvider::Tavily | SearchProvider::Serper => format!("{base}/search"),
        SearchProvider::DuckDuckGo => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let mut url = format!("{base}/html/?q={}", urlencoding::encode(query));
            if let Some(lr) = args.get("lr").and_then(Value::as_str) {
                url.push_str(&format!("&kl={}", urlencoding::encode(lr)));
            }
            url
        }
    }
}

struct JsonSearchRequest<'a> {
    endpoint: &'a str,
    query: &'a str,
    api_key: &'a str,
    max_results: usize,
    lr: Option<&'a str>,
    timeout_secs: u64,
    agent_scope: Option<&'a symbio_plugin_api::AgentScope>,
    policy: SandboxPolicy<'a>,
    trusted_origin: Option<url::Origin>,
}

/// Web 搜索工具
#[derive(Clone)]
pub struct WebSearchTool {
    config: Arc<RwLock<WebConfig>>,
    max_results: usize,
}

impl WebSearchTool {
    pub fn new(config: Arc<RwLock<WebConfig>>) -> Self {
        Self {
            config,
            max_results: DEFAULT_MAX_RESULTS,
        }
    }

    async fn execute_inner(
        &self,
        args: &Value,
        agent_scope: Option<&symbio_plugin_api::AgentScope>,
        sandbox: Option<&dyn SandboxFace>,
    ) -> PluginInvokeResponse<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'query' 参数".to_string()))?;

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_RESULTS_LIMIT))
            .unwrap_or(self.max_results);

        let lr: Option<String> = args
            .get("lr")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if query.trim().is_empty() {
            return Err(PluginError::ValidationError("搜索查询不能为空".to_string()));
        }

        // 配置在入口读一次并拷走：不把读锁带过 `await`——
        // 一次搜索最长可跑 `web_timeout` 秒，期间保存配置不该被堵住。
        let config = self.config.read().await.clone();
        let timeout_secs = config.web_timeout;
        let tavily_api_key = config.tavily_api_key.as_deref();
        let serper_api_key = config.serper_api_key.as_deref();
        let trusted_origin = if config.search_api_base.trim().is_empty() {
            None
        } else {
            Some(validate_upstream_http_url(&config.search_api_base)?.origin())
        };

        // ── 第三道闸：出站主机须在 Agent 声明的域名白名单里──
        //
        // 上游主机 = 「走哪家」×「有没有配 `search_api_base`」，三者都在上面这一块
        // 里定下来了，所以在分支**之前**算一次、判一次——三个分支各判一遍会得到
        // 三份同形代码，而漏一个分支就是「同一个 Agent，Tavily 能搜、Serper 不能」。
        // 数据照样经这个主机出境（查询内容 + API key），所以 Agent 域名授权不因
        // 「主机由宿主配置决定」而豁免；只有管理员显式配置的上游 origin 可用内网。
        let provider = selected_provider(&config);
        let upstream = upstream_url(&config, args);
        let policy = SandboxPolicy::new(
            sandbox,
            "web_search",
            args,
            symbio_plugin_api::CapabilityRiskLevel::Low,
        );

        // 优先使用 Tavily
        if provider == SearchProvider::Tavily {
            let api_key = tavily_api_key.unwrap_or_default().trim(); // selected_provider 已保证非空
            return match Self::search_tavily(JsonSearchRequest {
                endpoint: &upstream,
                query,
                api_key,
                max_results,
                lr: None,
                timeout_secs,
                agent_scope,
                policy,
                trusted_origin,
            })
            .await
            {
                Ok(results) => Ok(json!({
                    "success": true,
                    "results": results,
                    "provider": "tavily"
                })),
                Err(e) => Err(PluginError::InternalError(e)),
            };
        }

        // 其次使用 Serper
        if provider == SearchProvider::Serper {
            let api_key = serper_api_key.unwrap_or_default().trim(); // selected_provider 已保证非空
            return match Self::search_serper(JsonSearchRequest {
                endpoint: &upstream,
                query,
                api_key,
                max_results,
                lr: lr.as_deref(),
                timeout_secs,
                agent_scope,
                policy,
                trusted_origin,
            })
            .await
            {
                Ok(results) => Ok(json!({
                    "success": true,
                    "results": results,
                    "provider": "serper"
                })),
                Err(e) => Err(PluginError::InternalError(e)),
            };
        }

        // 最后回退到 DuckDuckGo (HTML 抓取)
        match self
            .search_duckduckgo(
                &upstream,
                max_results,
                timeout_secs,
                agent_scope,
                policy,
                trusted_origin,
            )
            .await
        {
            Ok(results) => Ok(json!({
                "success": true,
                "results": results,
                "provider": "duckduckgo"
            })),
            Err(e) => Err(PluginError::InternalError(e)),
        }
    }

    async fn search_tavily(request: JsonSearchRequest<'_>) -> Result<Vec<Value>, String> {
        let JsonSearchRequest {
            endpoint,
            query,
            api_key,
            max_results,
            timeout_secs,
            agent_scope,
            policy,
            trusted_origin,
            ..
        } = request;
        let endpoint = validate_upstream_http_url(endpoint).map_err(|error| error.to_string())?;
        let response = send_with_safe_redirects(
            endpoint,
            OutboundRequest {
                method: reqwest::Method::POST,
                headers: Vec::new(),
                body: Some(OutboundBody::Json(json!({
                    "api_key": api_key,
                    "query": query,
                    "max_results": max_results,
                    "search_depth": "basic"
                }))),
            },
            Duration::from_secs(timeout_secs),
            None,
            policy,
            trusted_origin,
            |host| agent_network_allowed(host, agent_scope),
        )
        .await
        .map_err(|error| error.to_string())?;

        if !response.status().is_success() {
            return Err(format!("Tavily 响应错误: {}", response.status()));
        }

        let (text, _, _) = read_response_text(response, MAX_RESPONSE_SIZE)
            .await
            .map_err(|error| error.to_string())?;
        let body: Value =
            serde_json::from_str(&text).map_err(|e| format!("解析 Tavily 响应失败: {e}"))?;

        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "Tavily 返回结果格式不正确".to_string())?;

        Ok(results
            .iter()
            .map(|r| {
                json!({
                    "title": r.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                    "url": r.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                    "snippet": r.get("content").and_then(|v| v.as_str()).unwrap_or("")
                })
            })
            .collect())
    }

    async fn search_serper(request: JsonSearchRequest<'_>) -> Result<Vec<Value>, String> {
        let JsonSearchRequest {
            endpoint,
            query,
            api_key,
            max_results,
            lr,
            timeout_secs,
            agent_scope,
            policy,
            trusted_origin,
        } = request;
        let endpoint = validate_upstream_http_url(endpoint).map_err(|error| error.to_string())?;
        let mut payload = json!({
            "q": query,
            "num": max_results
        });
        if let Some(l) = lr {
            payload["hl"] = json!(l);
        }

        let response = send_with_safe_redirects(
            endpoint,
            OutboundRequest {
                method: reqwest::Method::POST,
                headers: vec![("X-API-KEY".to_string(), api_key.to_string())],
                body: Some(OutboundBody::Json(payload)),
            },
            Duration::from_secs(timeout_secs),
            None,
            policy,
            trusted_origin,
            |host| agent_network_allowed(host, agent_scope),
        )
        .await
        .map_err(|error| error.to_string())?;

        if !response.status().is_success() {
            return Err(format!("Serper 响应错误: {}", response.status()));
        }

        let (text, _, _) = read_response_text(response, MAX_RESPONSE_SIZE)
            .await
            .map_err(|error| error.to_string())?;
        let body: Value =
            serde_json::from_str(&text).map_err(|e| format!("解析 Serper 响应失败: {e}"))?;

        let organic = body
            .get("organic")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "Serper 返回结果格式不正确".to_string())?;

        Ok(organic
            .iter()
            .map(|r| {
                json!({
                    "title": r.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                    "url": r.get("link").and_then(|v| v.as_str()).unwrap_or(""),
                    "snippet": r.get("snippet").and_then(|v| v.as_str()).unwrap_or("")
                })
            })
            .collect())
    }

    async fn search_duckduckgo(
        &self,
        endpoint: &str,
        max_results: usize,
        timeout_secs: u64,
        agent_scope: Option<&symbio_plugin_api::AgentScope>,
        policy: SandboxPolicy<'_>,
        trusted_origin: Option<url::Origin>,
    ) -> Result<Vec<Value>, String> {
        let endpoint = validate_upstream_http_url(endpoint).map_err(|error| error.to_string())?;
        let response = send_with_safe_redirects(
            endpoint,
            OutboundRequest {
                method: reqwest::Method::GET,
                headers: Vec::new(),
                body: None,
            },
            Duration::from_secs(timeout_secs),
            Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"),
            policy,
            trusted_origin,
            |host| agent_network_allowed(host, agent_scope),
        )
        .await
        .map_err(|error| error.to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "DuckDuckGo 搜索失败，状态码: {}",
                response.status()
            ));
        }

        let (html, _, _) = read_response_text(response, MAX_RESPONSE_SIZE)
            .await
            .map_err(|error| error.to_string())?;

        self.parse_duckduckgo_results(&html, max_results)
    }

    fn parse_duckduckgo_results(
        &self,
        html: &str,
        max_results: usize,
    ) -> Result<Vec<Value>, String> {
        let link_regex = Regex::new(
            r#"<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)</a>"#,
        )
        .map_err(|e| format!("编译正则表达式失败: {e}"))?;

        let snippet_regex = Regex::new(r#"<a class="result__snippet[^"]*"[^>]*>([\s\S]*?)</a>"#)
            .map_err(|e| format!("编译正则表达式失败: {e}"))?;

        let link_matches: Vec<_> = link_regex
            .captures_iter(html)
            .take(max_results + 2)
            .collect();

        let snippet_matches: Vec<_> = snippet_regex
            .captures_iter(html)
            .take(max_results + 2)
            .collect();

        if link_matches.is_empty() {
            return Ok(vec![]);
        }

        let mut results = Vec::new();
        for (i, cap) in link_matches.iter().enumerate() {
            if results.len() >= max_results {
                break;
            }

            let url = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let title = cap
                .get(2)
                .map(|m| clean_html(m.as_str()))
                .unwrap_or_default();

            if url.contains("duckduckgo.com") || url.is_empty() || title.is_empty() {
                continue;
            }

            let snippet = snippet_matches
                .get(i)
                .and_then(|s| s.get(1))
                .map(|m| clean_html(m.as_str()))
                .unwrap_or_default();

            results.push(json!({
                "title": title,
                "url": url,
                "snippet": snippet
            }));
        }

        Ok(results)
    }
}

impl WebSearchTool {
    pub fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "web_search".to_string(),
            description: "搜索互联网。使用 DuckDuckGo 进行搜索。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "搜索关键词"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "最大结果数（默认 5，最大 10）"
                    },
                    "lr": {
                        "type": "string",
                        "description": "语言/地区限制（如 zh-CN、en-US），对齐 Trae WebSearch 的 lr"
                    }
                },
                "required": ["query"]
            }),
            category: Some(symbio_plugin_api::CapabilityCategory::Network),
            // 只读检索 ⇒ 低风险（同 web_fetch）。依赖默认档会让它在 low 阈值下多要审批。
            risk: Some(symbio_plugin_api::CapabilityRiskLevel::Low),
            examples: Some(vec![
                "query='rust tutorial'".to_string(),
                "query='MODEL news', max_results=10".to_string(),
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
        let sandbox = symbio_plugin_api::sandbox_of(ctx.as_ref());
        let timeout_secs = self.config.read().await.web_timeout;
        let data = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            self.execute_inner(&args, scope.as_ref(), sandbox.as_deref()),
        )
        .await
        .map_err(|_| {
            PluginError::InternalError(format!("搜索超时：{timeout_secs}s 内未完成请求与响应解析"))
        })??;
        Ok(serde_json::to_value(&data)?)
    }
}

#[async_trait]
impl PortExecutor for WebSearchTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return missing_caller();
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

fn clean_html(html: &str) -> String {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
    let text = RE.replace_all(html, "");

    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "web_search.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/audits/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio-plugin-web/src/web_search.rs::clean_html: 编译期常量正则，模式写错在首次调用即崩——属开发期事故，不是运行期输入。
