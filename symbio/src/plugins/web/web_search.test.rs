//! `symbio/src/plugins/web/web_search.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

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
