//! `symbio/src/plugins/web/http_request.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

fn tool_with_domains(domains: &[&str]) -> HttpRequestTool {
    HttpRequestTool {
        allowed_domains: domains.iter().map(|s| s.to_string()).collect(),
        max_response_size: DEFAULT_MAX_RESPONSE_SIZE,
        timeout_secs: DEFAULT_TIMEOUT_SECS,
    }
}

/// SSRF 防护必须按 **IP 语义**判，不能按字符串前缀——
/// 前缀比较只认得出字面写死的 `127.0.0.1`，其余回环 / 内网形式全部放行。
#[test]
fn private_and_loopback_hosts_are_blocked() {
    for bad in [
        "localhost",
        "foo.localhost",
        "127.0.0.1",
        "127.0.0.2", // 整个 127/8 都是回环
        "0.0.0.0",
        "10.1.2.3",
        "192.168.1.1",
        "172.16.0.1",
        "172.31.255.255",
        "169.254.169.254", // 云元数据端点
        "::1",
        "::",
        "fc00::1",
        "fe80::1",
        "::ffff:127.0.0.1", // IPv4-mapped
        "printer.local",
        "db.internal",
    ] {
        assert!(is_private_or_local_host(bad), "{bad} 应被判为本地/私有");
    }
    for ok in [
        "example.com",
        "api.github.com",
        "8.8.8.8",
        "172.32.0.1", // 172.16/12 之外
        "2606:4700::1111",
    ] {
        assert!(!is_private_or_local_host(ok), "{ok} 不该被判为本地/私有");
    }
}

/// 通配白名单按**标签边界**匹配：`evil-example.com` 不能被 `*.example.com` 放行
#[test]
fn wildcard_allowlist_respects_label_boundary() {
    let allow = |h: &str| host_matches_allowlist(h, &["*.example.com".to_string()]);
    assert!(allow("example.com"), "裸域名本身应匹配");
    assert!(allow("api.example.com"));
    assert!(allow("a.b.example.com"));
    assert!(!allow("evil-example.com"), "后缀相同但不是子域");
    assert!(!allow("example.com.evil.net"));
    assert!(!allow("exampleXcom"));
}

#[test]
fn allowlist_is_case_insensitive_and_exact_without_wildcard() {
    let allow = |h: &str| host_matches_allowlist(h, &["API.GitHub.com".to_string()]);
    assert!(allow("api.github.com"));
    assert!(allow("API.github.COM"));
    assert!(!allow("evil.api.github.com"), "无通配项时只允许精确匹配");
    assert!(host_matches_allowlist(
        "anything.at.all",
        &["*".to_string()]
    ));
}

#[test]
fn validate_url_rejects_non_http_and_private_targets() {
    let t = tool_with_domains(&["*"]);
    assert!(t.validate_url("https://example.com/a").is_ok());
    for bad in [
        "",
        "ftp://example.com",
        "file:///etc/passwd",
        "https://exa mple.com",
        "http://127.0.0.1/admin",
        "http://169.254.169.254/latest/meta-data/",
        "http://[::1]:8080/",
    ] {
        assert!(t.validate_url(bad).is_err(), "应拒绝：{bad}");
    }
}

#[test]
fn validate_url_enforces_allowlist() {
    let t = tool_with_domains(&["*.example.com"]);
    assert!(t.validate_url("https://api.example.com/x").is_ok());
    assert!(t.validate_url("https://evil-example.com/x").is_err());
    assert!(t.validate_url("https://example.org/x").is_err());
}

#[test]
fn validate_method_accepts_known_and_rejects_unknown() {
    let t = HttpRequestTool::new();
    for m in ["GET", "post", "Put", "delete", "patch", "head", "options"] {
        assert!(t.validate_method(m).is_ok(), "{m}");
    }
    for bad in ["TRACE", "CONNECT", "", "GETY"] {
        assert!(t.validate_method(bad).is_err(), "{bad}");
    }
}
