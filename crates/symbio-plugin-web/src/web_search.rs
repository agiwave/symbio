//! Web 搜索工具 - 实现 Tool trait

use super::net_guard::{agent_network_allowed, extract_host};
use super::web_config::WebConfig;
use symbio_core::{
    Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest, PluginInvokeResponse,
};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS_LIMIT: usize = 10;

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
        agent_scope: Option<&symbio_core::AgentScope>,
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

        // 配置在入口读一次并拷走所需字段：不把读锁带过 `await`——
        // 一次搜索最长可跑 `web_timeout` 秒，期间保存配置不该被堵住。
        let (base, timeout_secs, tavily_api_key, serper_api_key) = {
            let cfg = self.config.read().await;
            (
                cfg.search_api_base.clone(),
                cfg.web_timeout,
                cfg.tavily_api_key.clone(),
                cfg.serper_api_key.clone(),
            )
        };

        // ── 第三道闸：出站主机须在 Agent 声明的域名白名单里（D-153）──
        //
        // 上游主机 = 「走哪家」×「有没有配 `search_api_base`」，三者都在上面这一块
        // 里定下来了，所以在分支**之前**算一次、判一次——三个分支各判一遍会得到
        // 三份同形代码，而漏一个分支就是「同一个 Agent，Tavily 能搜、Serper 不能」。
        // 数据照样经这个主机出境（查询内容 + API key），所以它和 `web_fetch` 的
        // 模型自选主机同案，不因「主机由宿主配置决定」而豁免。
        let official = if tavily_api_key.is_some() {
            TAVILY_OFFICIAL_BASE
        } else if serper_api_key.is_some() {
            SERPER_OFFICIAL_BASE
        } else {
            DUCKDUCKGO_OFFICIAL_BASE
        };
        let upstream = format!("{}/search", upstream_base(&base, official));
        agent_network_allowed(&extract_host(&upstream)?, agent_scope)?;

        // 优先使用 Tavily
        if let Some(api_key) = tavily_api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            return match self
                .search_tavily(
                    &base,
                    query,
                    api_key,
                    max_results,
                    lr.as_deref(),
                    timeout_secs,
                )
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
        if let Some(api_key) = serper_api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            return match self
                .search_serper(
                    &base,
                    query,
                    api_key,
                    max_results,
                    lr.as_deref(),
                    timeout_secs,
                )
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
            .search_duckduckgo(&base, query, max_results, lr.as_deref(), timeout_secs)
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

    async fn search_tavily(
        &self,
        base: &str,
        query: &str,
        api_key: &str,
        max_results: usize,
        _lr: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Vec<Value>, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

        let response = client
            .post(format!(
                "{}/search",
                upstream_base(base, TAVILY_OFFICIAL_BASE)
            ))
            .json(&json!({
                "api_key": api_key,
                "query": query,
                "max_results": max_results,
                "search_depth": "basic"
            }))
            .send()
            .await
            .map_err(|e| format!("Tavily 请求失败: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("Tavily 响应错误: {}", response.status()));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("解析 Tavily 响应失败: {e}"))?;

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

    async fn search_serper(
        &self,
        base: &str,
        query: &str,
        api_key: &str,
        max_results: usize,
        lr: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Vec<Value>, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

        let mut payload = json!({
            "q": query,
            "num": max_results
        });
        if let Some(l) = lr {
            payload["hl"] = json!(l);
        }

        let response = client
            .post(format!(
                "{}/search",
                upstream_base(base, SERPER_OFFICIAL_BASE)
            ))
            .header("X-API-KEY", api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Serper 请求失败: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("Serper 响应错误: {}", response.status()));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("解析 Serper 响应失败: {e}"))?;

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
        base: &str,
        query: &str,
        max_results: usize,
        lr: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Vec<Value>, String> {
        let encoded_query = urlencoding::encode(query);
        let mut search_url = format!(
            "{}/html/?q={encoded_query}",
            upstream_base(base, DUCKDUCKGO_OFFICIAL_BASE)
        );
        if let Some(l) = lr {
            search_url.push_str(&format!("&kl={}", urlencoding::encode(l)));
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

        let response = client
            .get(&search_url)
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "DuckDuckGo 搜索失败，状态码: {}",
                response.status()
            ));
        }

        let html = response
            .text()
            .await
            .map_err(|e| format!("读取响应失败: {e}"))?;

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

#[async_trait]
impl Capability for WebSearchTool {
    fn meta(&self) -> CapabilityMeta {
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
            category: Some(symbio_core::CapabilityCategory::Network),
            // 只读检索 ⇒ 低风险（同 web_fetch）。依赖默认档会让它在 low 阈值下多要审批。
            risk: Some(symbio_core::CapabilityRiskLevel::Low),
            examples: Some(vec![
                "query='rust tutorial'".to_string(),
                "query='MODEL news', max_results=10".to_string(),
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
        let scope = symbio_core::agent_perms_of(ctx.as_ref());
        let data = self.execute_inner(&args, scope.as_ref()).await?;
        Ok(serde_json::to_value(&data)?)
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
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio-plugin-web/src/web_search.rs::clean_html: 编译期常量正则，模式写错在首次调用即崩——属开发期事故，不是运行期输入。
