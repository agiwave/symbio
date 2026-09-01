//! Web 搜索工具 - 实现 Tool trait

use crate::symbio_core::schemas::web::web_config::WebConfig;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

const DEFAULT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS_LIMIT: usize = 10;

/// Web 搜索工具
#[derive(Clone)]
pub struct WebSearchTool {
    config: Arc<RwLock<WebConfig>>,
    max_results: usize,
    timeout_secs: u64,
}

impl WebSearchTool {
    pub fn new(config: Arc<RwLock<WebConfig>>) -> Self {
        Self {
            config,
            max_results: DEFAULT_MAX_RESULTS,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    async fn execute_inner(&self, args: &Value) -> InvokeResponse<Value> {
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

        let cfg = self.config.read().await;

        // 优先使用 Tavily
        if let Some(ref api_key) = cfg.tavily_api_key {
            if !api_key.trim().is_empty() {
                return match self
                    .search_tavily(query, api_key, max_results, lr.as_deref())
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
        }

        // 其次使用 Serper
        if let Some(ref api_key) = cfg.serper_api_key {
            if !api_key.trim().is_empty() {
                return match self
                    .search_serper(query, api_key, max_results, lr.as_deref())
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
        }

        // 最后回退到 DuckDuckGo (HTML 抓取)
        match self
            .search_duckduckgo(query, max_results, lr.as_deref())
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
        query: &str,
        api_key: &str,
        max_results: usize,
        _lr: Option<&str>,
    ) -> Result<Vec<Value>, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

        let response = client
            .post("https://api.tavily.com/search")
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
        query: &str,
        api_key: &str,
        max_results: usize,
        lr: Option<&str>,
    ) -> Result<Vec<Value>, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
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
            .post("https://google.serper.dev/search")
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
        query: &str,
        max_results: usize,
        lr: Option<&str>,
    ) -> Result<Vec<Value>, String> {
        let encoded_query = urlencoding::encode(query);
        let mut search_url = format!("https://html.duckduckgo.com/html/?q={encoded_query}");
        if let Some(l) = lr {
            search_url.push_str(&format!("&kl={}", urlencoding::encode(l)));
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
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
            category: Some(crate::symbio_core::CapabilityCategory::Network),
            examples: Some(vec![
                "query='rust tutorial'".to_string(),
                "query='MODEL news', max_results=10".to_string(),
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
