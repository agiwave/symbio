//! `crates/symbio-plugin-web/src/web_search.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use symbio_plugin_api::{SandboxDecision, SandboxFact};

fn tool() -> WebSearchTool {
    WebSearchTool::new(Arc::new(RwLock::new(WebConfig::default())))
}

#[test]
fn clean_html_strips_tags_and_decodes_entities() {
    assert_eq!(clean_html("<b>hi</b>"), "hi");
    assert_eq!(clean_html("a &amp; b"), "a & b");
    assert_eq!(clean_html("<p>a</p>\n\n   <p>b</p>"), "a b");
    assert_eq!(clean_html("&lt;script&gt;"), "<script>");
    assert_eq!(clean_html("  spaced   out  "), "spaced out");
    assert_eq!(clean_html(""), "");
}

/// 结果解析要同时做两件事：抽字段、滤掉 DuckDuckGo 自己的跳转链接
#[test]
fn duckduckgo_parser_extracts_fields_and_filters_self_links() {
    let t = tool();
    let html = r#"
        <a class="result__a" href="https://rust-lang.org/">Rust &amp; <b>Cargo</b></a>
        <a class="result__snippet">The Rust programming language</a>
        <a class="result__a" href="https://duckduckgo.com/y.js?ad=1">广告</a>
        <a class="result__a" href="https://docs.rs/">docs.rs</a>
        <a class="result__snippet">crate documentation</a>
    "#;
    let out = t.parse_duckduckgo_results(html, 5).unwrap();
    assert_eq!(out.len(), 2, "ddg 自身链接必须被过滤：{out:?}");
    assert_eq!(out[0]["url"], "https://rust-lang.org/");
    assert_eq!(out[0]["title"], "Rust & Cargo");
    assert_eq!(out[0]["snippet"], "The Rust programming language");
    assert_eq!(out[1]["url"], "https://docs.rs/");
}

#[test]
fn duckduckgo_parser_honours_max_results() {
    let t = tool();
    let html: String = (0..10)
        .map(|i| format!(r#"<a class="result__a" href="https://ex{i}.com/">t{i}</a>"#))
        .collect();
    assert_eq!(t.parse_duckduckgo_results(&html, 3).unwrap().len(), 3);
}

#[test]
fn duckduckgo_parser_returns_empty_on_unrecognized_html() {
    let t = tool();
    let out = t.parse_duckduckgo_results("<html>nothing here</html>", 5);
    assert!(out.unwrap().is_empty());
}

/// `search_api_base` 为空（缺省）⇒ 官方端点——**对现有部署零行为变化**是加这一位的前提
#[test]
fn upstream_base_defaults_to_official_endpoints() {
    assert_eq!(
        upstream_base("", TAVILY_OFFICIAL_BASE),
        "https://api.tavily.com"
    );
    assert_eq!(
        upstream_base("", SERPER_OFFICIAL_BASE),
        "https://google.serper.dev"
    );
    assert_eq!(
        upstream_base("", DUCKDUCKGO_OFFICIAL_BASE),
        "https://html.duckduckgo.com"
    );
    // 纯空白同理按缺省处理
    assert_eq!(
        upstream_base("  ", TAVILY_OFFICIAL_BASE),
        "https://api.tavily.com"
    );
}

/// 配置了基址 ⇒ 三家统一替换，且**尾随 `/` 被剥**（路径由调用方拼，双斜杠会 404）
#[test]
fn upstream_base_override_replaces_host_and_trims_slash() {
    assert_eq!(
        upstream_base("http://127.0.0.1:18090", TAVILY_OFFICIAL_BASE),
        "http://127.0.0.1:18090"
    );
    assert_eq!(
        upstream_base("http://127.0.0.1:18090/", SERPER_OFFICIAL_BASE),
        "http://127.0.0.1:18090"
    );
    assert_eq!(
        upstream_base("http://127.0.0.1:18090///", DUCKDUCKGO_OFFICIAL_BASE),
        "http://127.0.0.1:18090"
    );
}

#[test]
fn url_facts_use_the_selected_providers_final_endpoint() {
    let tavily = WebConfig {
        tavily_api_key: Some("tavily-secret".to_string()),
        serper_api_key: Some("serper-secret".to_string()),
        ..WebConfig::default()
    };
    let serper = WebConfig {
        serper_api_key: Some("serper-secret".to_string()),
        ..WebConfig::default()
    };
    let args = json!({"query": "rust news"});

    assert_eq!(
        upstream_url(&tavily, &args),
        "https://api.tavily.com/search"
    );
    assert_eq!(
        upstream_url(&serper, &args),
        "https://google.serper.dev/search"
    );
    assert_eq!(
        upstream_url(&WebConfig::default(), &args),
        "https://html.duckduckgo.com/html/?q=rust%20news"
    );
}

#[test]
fn configured_base_keeps_provider_path_and_never_exposes_api_key() {
    let config = WebConfig {
        tavily_api_key: Some("  top-secret  ".to_string()),
        search_api_base: "https://proxy.example/root///".to_string(),
        ..WebConfig::default()
    };
    let tavily = upstream_url(&config, &json!({"query": "q"}));
    assert_eq!(tavily, "https://proxy.example/root/search");
    assert!(!tavily.contains("top-secret"));

    let serper = WebConfig {
        tavily_api_key: Some("   ".to_string()),
        serper_api_key: Some("serper-secret".to_string()),
        search_api_base: "https://proxy.example/".to_string(),
        ..WebConfig::default()
    };
    let endpoint = upstream_url(&serper, &json!({"query": "q"}));
    assert_eq!(endpoint, "https://proxy.example/search");
    assert!(!endpoint.contains("serper-secret"));

    let ddg = WebConfig {
        search_api_base: "https://proxy.example/".to_string(),
        ..WebConfig::default()
    };
    assert_eq!(
        upstream_url(&ddg, &json!({"query": "rust & security", "lr": "zh-CN"})),
        "https://proxy.example/html/?q=rust%20%26%20security&kl=zh-CN"
    );
}

struct DenyingSandbox {
    calls: std::sync::atomic::AtomicUsize,
}

impl SandboxFace for DenyingSandbox {
    fn permits(&self, _facts: &[SandboxFact]) -> bool {
        false
    }

    fn intercept(&self, _tool_name: &str, _args: &Value) -> Option<Value> {
        None
    }

    fn decide(&self, facts: &[SandboxFact], tool_name: &str, args: &Value) -> SandboxDecision {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(tool_name, "web_search");
        assert_eq!(args, &json!({"query": "guarded"}));
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].kind, "url");
        SandboxDecision {
            permitted: false,
            reason: Some("test denial".into()),
            ..SandboxDecision::default()
        }
    }
}

#[tokio::test]
async fn every_search_provider_is_denied_before_dns_or_connect() {
    for config in [
        WebConfig {
            tavily_api_key: Some("tavily-secret".into()),
            search_api_base: "https://guarded.example".into(),
            ..WebConfig::default()
        },
        WebConfig {
            serper_api_key: Some("serper-secret".into()),
            search_api_base: "https://guarded.example".into(),
            ..WebConfig::default()
        },
        WebConfig {
            search_api_base: "https://guarded.example".into(),
            ..WebConfig::default()
        },
    ] {
        let sandbox = DenyingSandbox {
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let tool = WebSearchTool::new(Arc::new(RwLock::new(config)));
        let error = tool
            .execute_inner(&json!({"query": "guarded"}), None, Some(&sandbox))
            .await
            .expect_err("沙箱拒绝必须终止搜索");
        assert!(error.to_string().contains("test denial"), "{error}");
        assert_eq!(sandbox.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
