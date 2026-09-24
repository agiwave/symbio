//! `symbio/src/plugins/model/protocols/context_probe.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn server_root_strips_v1_suffix() {
    assert_eq!(
        server_root("http://localhost:11434/v1"),
        "http://localhost:11434"
    );
    assert_eq!(
        server_root("http://localhost:1234/v1/"),
        "http://localhost:1234"
    );
    // 无 /v1 后缀：保持原样
    assert_eq!(
        server_root("http://localhost:8080"),
        "http://localhost:8080"
    );
}

#[test]
fn ollama_name_matches_strips_latest_tag() {
    assert!(ollama_name_matches("qwen2.5:latest", "qwen2.5"));
    assert!(ollama_name_matches("qwen2.5", "qwen2.5"));
    assert!(ollama_name_matches("qwen2.5:7b", "qwen2.5:7b"));
    assert!(!ollama_name_matches("qwen2.5:7b", "qwen2.5"));
}

#[tokio::test]
async fn cached_probe_caches_none_results() {
    let key = "test://none-case";
    let first = cached_probe(key, async { None }).await;
    assert_eq!(first, None);
    // 第二次命中缓存：探测 future 不应再被调用（若被调用会 panic）
    let second = cached_probe(key, async { panic!("should hit cache") }).await;
    assert_eq!(second, None);
}

#[tokio::test]
async fn cached_probe_caches_value() {
    let key = "test://value-case";
    let first = cached_probe(key, async { Some(8192) }).await;
    assert_eq!(first, Some(8192));
    let second = cached_probe(key, async { panic!("should hit cache") }).await;
    assert_eq!(second, Some(8192));
}
