//! Web 搜索工具 - 实现 Plugin trait

use crate::symbio_core::traits::Plugin;
use crate::symbio_core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS_LIMIT: usize = 10;

/// Web 搜索工具
pub struct WebSearchTool {
    max_results: usize,
    timeout_secs: u64,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            max_results: DEFAULT_MAX_RESULTS,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "web_search".to_string(),
            description: "搜索互联网。使用 DuckDuckGo 进行搜索。".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "搜索关键词"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "最大结果数（默认5，最大10）"
                    }
                },
                "required": ["query"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "results": { "type": "array" },
                    "provider": { "type": "string" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    async fn execute_inner(&self, args: &Value) -> Result<StreamChunk, PluginError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'query' 参数".to_string()))?;

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_RESULTS_LIMIT))
            .unwrap_or(self.max_results);

        if query.trim().is_empty() {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some("搜索查询不能为空".to_string()),
            });
        }

        // 使用 DuckDuckGo 搜索
        match self.search_duckduckgo(query, max_results).await {
            Ok(results) => Ok(StreamChunk {
                data: json!({
                    "success": true,
                    "results": results,
                    "provider": "duckduckgo"
                }),
                done: true,
                error: None,
            }),
            Err(e) => Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some(e),
            }),
        }
    }

    async fn search_duckduckgo(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<Value>, String> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://html.duckduckgo.com/html/?q={}", encoded_query);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        let response = client
            .get(&search_url)
            .send()
            .await
            .map_err(|e| format!("请求失败: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("DuckDuckGo 搜索失败，状态码: {}", response.status()));
        }

        let html = response
            .text()
            .await
            .map_err(|e| format!("读取响应失败: {}", e))?;

        self.parse_duckduckgo_results(&html, max_results)
    }

    fn parse_duckduckgo_results(&self, html: &str, max_results: usize) -> Result<Vec<Value>, String> {
        let link_regex = Regex::new(
            r#"<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)</a>"#,
        )
        .map_err(|e| format!("编译正则表达式失败: {}", e))?;

        let snippet_regex = Regex::new(r#"<a class="result__snippet[^"]*"[^>]*>([\s\S]*?)</a>"#)
            .map_err(|e| format!("编译正则表达式失败: {}", e))?;

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
            let title = cap.get(2).map(|m| clean_html(m.as_str())).unwrap_or_default();
            
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
impl Plugin for WebSearchTool {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(Self::create_meta())
        } else {
            Err(PluginError::NotFound(format!("路径不存在: {}", path)))
        }
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if !path.is_empty() {
            return Err(PluginError::NotFound(format!("路径不存在: {}", path)));
        }

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.execute_inner(&input).await
            })
        })?;

        Ok(InvokeStream::Single(result))
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

fn clean_html(html: &str) -> String {
    let re = Regex::new(r"<[^>]*>").unwrap();
    let text = re.replace_all(html, "");
    
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    
    text.trim().split_whitespace().collect::<Vec<_>>().join(" ")
}