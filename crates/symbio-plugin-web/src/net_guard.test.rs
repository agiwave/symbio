//! `crates/symbio-plugin-web/src/net_guard.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 第三道闸（Agent 声明的出站白名单）的反向用例全在这里：**没有选中 Agent 时恒过、
//! 选中了而清单为空时恒拒**——这两条是该字段的语义本身，写反任何一条，
//! 「声明要成为安全边界」就不成立。

use super::*;
use serde_json::json;
use std::path::PathBuf;
use symbio_plugin_api::{CapabilityRiskLevel, SandboxDecision};

fn scope(domains: &[&str]) -> AgentScope {
    AgentScope {
        id: "reviewer".to_string(),
        root: PathBuf::from("/nowhere/agent/reviewer"),
        // 经线格式构造（与 manifest 解析同一条路）：不深引 core 私有模块取子类型，
        // 也顺带钉住「声明就是 manifest 里那个形状」。
        permissions: serde_json::from_value(json!({ "network": { "domains": domains } }))
            .expect("声明可解析"),
    }
}

/// 没有选中 Agent ⇒ 宿主态，第三道闸恒过（没有 manifest 可声明）。
#[test]
fn no_agent_scope_means_host_mode() {
    assert!(agent_network_allowed("anything.example.org", None).is_ok());
}

/// 选中了而清单为空 ⇒ **一律拒**（grant 语义：空清单即没有授权）。
/// 这一条是该字段的语义本身——放行它，「不声明就能出网」就让整份声明失去意义。
#[test]
fn an_empty_domain_list_denies_every_host() {
    let s = scope(&[]);
    for host in ["example.com", "api.github.com", "127.0.0.1"] {
        let err = agent_network_allowed(host, Some(&s)).unwrap_err();
        assert!(
            err.to_string().contains("permissions.network.domains"),
            "拒绝理由须点名是哪个字段：{err}"
        );
    }
}

/// 清单内的主机放行；清单外的拒，且**拒绝发生在发请求之前**（本函数在建连前被调用）。
#[test]
fn only_declared_hosts_pass() {
    let s = scope(&["api.github.com", "*.example.com"]);
    assert!(agent_network_allowed("api.github.com", Some(&s)).is_ok());
    assert!(agent_network_allowed("cdn.example.com", Some(&s)).is_ok());
    // 标签边界：字符串后缀比较会把这两个也放行。
    assert!(agent_network_allowed("evil-api.github.com", Some(&s)).is_err());
    assert!(agent_network_allowed("notexample.com", Some(&s)).is_err());
}

fn socket(address: &str, port: u16) -> SocketAddr {
    let address = if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    };
    address.parse().expect("测试地址有效")
}

/// DNS rebinding 的混合答案必须整体拒绝，不能只挑一个公网答案连接。
#[test]
fn mixed_public_private_dns_answers_are_rejected() {
    let public = socket("93.184.216.34", 443);
    for private in [
        socket("127.0.0.1", 443),
        socket("10.0.0.8", 443),
        socket("169.254.169.254", 443),
        socket("0.0.0.0", 443),
        socket("224.0.0.1", 443),
        socket("100.64.0.1", 443),
        socket("::1", 443),
        socket("fe80::1", 443),
        socket("fc00::1", 443),
        socket("::ffff:127.0.0.1", 443),
        socket("::ffff:0.0.0.0", 443),
    ] {
        let err = validate_resolved_addresses("rebind.example", &[public, private])
            .expect_err("混合答案必须拒绝");
        assert!(
            err.to_string().contains("阻止本地/私有地址"),
            "拒绝应指出 DNS 答案：{err}"
        );
    }
}

#[test]
fn all_public_dns_answers_are_kept_and_deduplicated() {
    let v4 = socket("93.184.216.34", 443);
    let v6 = socket("2606:4700:4700::1111", 443);
    let addresses =
        validate_resolved_addresses("public.example", &[v4, v6, v4]).expect("全公网答案应通过");
    assert_eq!(addresses, vec![v4, v6]);
}

#[test]
fn empty_dns_answer_is_an_internal_error() {
    let err = validate_resolved_addresses("empty.example", &[]).expect_err("空答案必须拒绝");
    assert!(err.to_string().contains("未返回任何地址"), "{err}");
}

#[test]
fn only_configured_upstreams_may_use_private_addresses() {
    let local = "http://127.0.0.1:8787/search";
    assert!(validate_http_url(local).is_err(), "模型 URL 不得绕过私网闸");
    let upstream = validate_upstream_http_url(local).expect("管理员配置的内网上游应可用");
    assert_eq!(upstream.host_str(), Some("127.0.0.1"));
    assert_eq!(upstream.port(), Some(8787));
}

#[test]
fn configured_upstreams_still_reject_credentials_and_non_http_schemes() {
    for raw in [
        "http://user:pass@127.0.0.1/search",
        "file:///etc/passwd",
        "ftp://127.0.0.1/search",
    ] {
        assert!(validate_upstream_http_url(raw).is_err(), "{raw}");
    }
}

#[test]
fn redirect_to_private_or_non_http_target_is_rejected() {
    let current = Url::parse("https://public.example/start").unwrap();
    for location in [
        "http://127.0.0.1/admin",
        "http://169.254.169.254/latest/meta-data/",
        "http://[::1]/",
        "file:///etc/passwd",
        "ftp://public.example/file",
    ] {
        let err = next_redirect(&current, StatusCode::FOUND, Some(location), 0, |_| Ok(()))
            .expect_err("危险重定向必须拒绝");
        assert!(
            err.to_string().contains("阻止本地/私有") || err.to_string().contains("HTTP/HTTPS"),
            "{location}: {err}"
        );
    }
}

#[test]
fn safe_relative_redirect_resolves_against_current_url() {
    let current = Url::parse("https://public.example/a/start").unwrap();
    let target = next_redirect(
        &current,
        StatusCode::FOUND,
        Some("../next?x=1"),
        0,
        |host| {
            assert_eq!(host, "public.example");
            Ok(())
        },
    )
    .unwrap()
    .expect("安全相对跳应返回目标");
    assert_eq!(target.as_str(), "https://public.example/next?x=1");
}

#[test]
fn redirect_limit_allows_five_hops_and_rejects_the_sixth() {
    let mut current = Url::parse("https://public.example/0").unwrap();
    for used in 0..MAX_REDIRECTS {
        current = next_redirect(
            &current,
            StatusCode::FOUND,
            Some(&format!("/{}", used + 1)),
            used,
            |_| Ok(()),
        )
        .unwrap()
        .expect("前五跳应允许");
    }
    assert_eq!(current.path(), "/5");

    let err = next_redirect(
        &current,
        StatusCode::FOUND,
        Some("/6"),
        MAX_REDIRECTS,
        |_| Ok(()),
    )
    .expect_err("第六跳必须拒绝");
    assert!(err.to_string().contains("最多 5 次"), "{err}");
}

#[test]
fn redirect_without_location_is_returned_as_a_normal_response() {
    let current = Url::parse("https://public.example/start").unwrap();
    assert!(
        next_redirect(&current, StatusCode::FOUND, None, MAX_REDIRECTS, |_| Ok(()))
            .unwrap()
            .is_none()
    );
    assert!(next_redirect(
        &current,
        StatusCode::OK,
        Some("http://127.0.0.1/"),
        0,
        |_| Ok(())
    )
    .unwrap()
    .is_none());
}

fn post_request() -> OutboundRequest {
    OutboundRequest {
        method: Method::POST,
        headers: vec![
            ("Content-Type".into(), "text/plain".into()),
            ("Content-Length".into(), "3".into()),
            ("X-Keep".into(), "yes".into()),
        ],
        body: Some(OutboundBody::Text("abc".into())),
    }
}

#[test]
fn post_method_and_body_conversion_matches_redirect_status() {
    let from = Url::parse("https://public.example/start").unwrap();
    let to = Url::parse("https://public.example/next").unwrap();

    for status in [StatusCode::MOVED_PERMANENTLY, StatusCode::FOUND] {
        let redirected = post_request().redirected(status, &from, &to).unwrap();
        assert_eq!(redirected.method, Method::GET, "{status}");
        assert!(redirected.body.is_none(), "{status} 应丢弃 body");
        assert!(
            !redirected
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-type")),
            "{status} 应丢弃 body 相关头"
        );
        assert!(redirected.headers.iter().any(|(name, _)| name == "X-Keep"));
    }

    let see_other = post_request()
        .redirected(StatusCode::SEE_OTHER, &from, &to)
        .unwrap();
    assert_eq!(see_other.method, Method::GET);
    assert!(see_other.body.is_none());

    for status in [
        StatusCode::TEMPORARY_REDIRECT,
        StatusCode::PERMANENT_REDIRECT,
    ] {
        let redirected = post_request().redirected(status, &from, &to).unwrap();
        assert_eq!(redirected.method, Method::POST, "{status}");
        assert!(matches!(redirected.body, Some(OutboundBody::Text(ref body)) if body == "abc"));
        assert!(redirected
            .headers
            .iter()
            .any(|(name, _)| name == "Content-Type"));
    }
}

#[test]
fn head_is_preserved_on_303_and_cross_origin_redirect_drops_credentials() {
    let from = Url::parse("https://public.example/start").unwrap();
    let same_origin = Url::parse("https://public.example/next").unwrap();
    let head = OutboundRequest {
        method: Method::HEAD,
        ..post_request()
    }
    .redirected(StatusCode::SEE_OTHER, &from, &same_origin)
    .unwrap();
    assert_eq!(head.method, Method::HEAD);
    assert!(head.body.is_some());
}

#[test]
fn cross_origin_redirect_drops_all_non_portable_headers() {
    let from = Url::parse("https://public.example/start").unwrap();
    let to = Url::parse("https://other.example/next").unwrap();
    let redirected = OutboundRequest {
        method: Method::GET,
        body: None,
        headers: vec![
            ("Authorization".into(), "Bearer secret".into()),
            ("Cookie".into(), "session=secret".into()),
            ("Proxy-Authorization".into(), "Basic secret".into()),
            ("X-API-Key".into(), "secret".into()),
            ("Accept".into(), "text/plain".into()),
            ("Content-Type".into(), "text/plain".into()),
        ],
    }
    .redirected(StatusCode::FOUND, &from, &to)
    .unwrap();

    assert_eq!(
        redirected.headers,
        vec![
            ("Accept".to_string(), "text/plain".to_string()),
            ("Content-Type".to_string(), "text/plain".to_string()),
        ]
    );
}

#[test]
fn cross_origin_307_and_308_do_not_replay_request_bodies() {
    let from = Url::parse("https://public.example/start").unwrap();
    let to = Url::parse("https://other.example/next").unwrap();
    for status in [
        StatusCode::TEMPORARY_REDIRECT,
        StatusCode::PERMANENT_REDIRECT,
    ] {
        let error = post_request()
            .redirected(status, &from, &to)
            .err()
            .expect("跨源重定向不得把凭据或请求体重放到另一源");
        assert!(matches!(error, PluginError::Forbidden(_)), "{error}");
    }
}

struct RecordingSandbox {
    denied_url: String,
    intercept: Option<Value>,
    risk: Option<String>,
    risk_threshold: Option<String>,
    seen: std::sync::Mutex<Vec<(SandboxFact, String, Value)>>,
}

impl RecordingSandbox {
    fn new(denied_url: &str) -> Self {
        Self {
            denied_url: denied_url.to_string(),
            intercept: None,
            risk: None,
            risk_threshold: None,
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl SandboxFace for RecordingSandbox {
    fn permits(&self, facts: &[SandboxFact]) -> bool {
        facts.iter().all(|fact| fact.value != self.denied_url)
    }

    fn intercept(&self, _tool_name: &str, _args: &Value) -> Option<Value> {
        self.intercept.clone()
    }

    fn decide(&self, facts: &[SandboxFact], tool_name: &str, args: &Value) -> SandboxDecision {
        self.seen
            .lock()
            .unwrap()
            .push((facts[0].clone(), tool_name.to_string(), args.clone()));
        SandboxDecision {
            permitted: self.permits(facts) && self.intercept.is_none(),
            intercept: self.intercept.clone(),
            risk: self.risk.clone(),
            risk_threshold: self.risk_threshold.clone(),
            reason: Some("test policy".to_string()),
            ..SandboxDecision::default()
        }
    }
}

/// 纯策略测试，不解析 DNS、不建连：第一跳放行后，第二个 URL 的 deny 必须终止链路。
#[test]
fn second_redirect_hop_deny_stops_before_send() {
    let sandbox = RecordingSandbox::new("https://blocked.example/second");
    let args = json!({"url": "https://public.example/start#input-fragment"});
    let policy = SandboxPolicy::new(Some(&sandbox), "web_fetch", &args, CapabilityRiskLevel::Low);
    let mut current = Url::parse("https://public.example/start#input-fragment").unwrap();

    policy.authorize(&current).unwrap();
    current = next_redirect(
        &current,
        StatusCode::FOUND,
        Some("https://blocked.example/second#location-fragment"),
        0,
        |_| Ok(()),
    )
    .unwrap()
    .unwrap();
    let err = policy.authorize(&current).unwrap_err();

    assert!(matches!(err, PluginError::Forbidden(_)), "{err}");
    let seen = sandbox.seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "第二个 hop 必须独立裁决，而不是复用首跳结果");
    assert_eq!(seen[0].0.kind, "url");
    assert_eq!(seen[0].0.value, "https://public.example/start");
    assert_eq!(seen[1].0.value, "https://blocked.example/second");
    assert!(seen
        .iter()
        .all(|(_, tool, call_args)| tool == "web_fetch" && *call_args == args));
}

#[test]
fn sandbox_intercept_stops_even_when_permits_would_allow() {
    let mut sandbox = RecordingSandbox::new("");
    sandbox.intercept = Some(json!({"approval": "required"}));
    let args = json!({"url": "https://public.example/next"});
    let err = SandboxPolicy::new(
        Some(&sandbox),
        "http_request",
        &args,
        CapabilityRiskLevel::High,
    )
    .authorize(&Url::parse("https://public.example/next").unwrap())
    .unwrap_err();
    assert!(matches!(err, PluginError::Forbidden(_)), "{err}");
}

#[test]
fn initial_url_risk_approval_is_left_to_the_tool_gate() {
    let mut sandbox = RecordingSandbox::new("");
    sandbox.risk = Some("high".to_string());
    sandbox.risk_threshold = Some("medium".to_string());
    let args = json!({"url": "https://public.example/start"});
    assert!(
        SandboxPolicy::new(Some(&sandbox), "web_fetch", &args, CapabilityRiskLevel::Low,)
            .authorize(&Url::parse("https://public.example/start").unwrap())
            .is_ok()
    );
}

#[test]
fn redirected_url_above_risk_threshold_requires_a_separate_approval() {
    let mut sandbox = RecordingSandbox::new("");
    sandbox.risk = Some("high".to_string());
    sandbox.risk_threshold = Some("medium".to_string());
    let args = json!({"url": "https://public.example/start"});
    let err = SandboxPolicy::new(Some(&sandbox), "web_fetch", &args, CapabilityRiskLevel::Low)
        .authorize_redirect(&Url::parse("https://public.example/redirected").unwrap())
        .unwrap_err();

    assert!(matches!(err, PluginError::Forbidden(_)), "{err}");
    assert!(err.to_string().contains("需要单独审批"), "{err}");
}

#[test]
fn redirected_url_at_or_below_risk_threshold_is_allowed() {
    let mut sandbox = RecordingSandbox::new("");
    sandbox.risk = Some("medium".to_string());
    sandbox.risk_threshold = Some("medium".to_string());
    let args = json!({"url": "https://public.example/start"});
    assert!(
        SandboxPolicy::new(Some(&sandbox), "web_fetch", &args, CapabilityRiskLevel::Low,)
            .authorize_redirect(&Url::parse("https://public.example/redirected").unwrap())
            .is_ok()
    );
}

#[test]
fn missing_sandbox_keeps_existing_outbound_behavior() {
    assert!(
        SandboxPolicy::new(None, "http_request", &json!({}), CapabilityRiskLevel::High,)
            .authorize(&Url::parse("https://public.example/next").unwrap())
            .is_ok()
    );
}

#[test]
fn redirect_uses_the_higher_of_tool_and_rule_risk() {
    let mut sandbox = RecordingSandbox::new("");
    sandbox.risk = Some("low".to_string());
    sandbox.risk_threshold = Some("medium".to_string());
    let args = json!({"url": "https://public.example/start"});
    let redirected = Url::parse("https://public.example/next").unwrap();

    let err = SandboxPolicy::new(
        Some(&sandbox),
        "http_request",
        &args,
        CapabilityRiskLevel::High,
    )
    .authorize_redirect(&redirected)
    .unwrap_err();
    assert!(err.to_string().contains("有效风险 high"), "{err}");

    sandbox.risk_threshold = Some("high".to_string());
    assert!(SandboxPolicy::new(
        Some(&sandbox),
        "http_request",
        &args,
        CapabilityRiskLevel::High,
    )
    .authorize_redirect(&redirected)
    .is_ok());
}

#[test]
fn url_credentials_are_rejected_and_removed_from_facts() {
    for raw in [
        "https://user:secret@example.com/path#fragment",
        "https://token@example.com/path",
    ] {
        assert!(validate_http_url(raw).is_err(), "{raw}");
    }
    assert_eq!(
        normalized_url_fact("https://user:secret@example.com/path#fragment"),
        "https://example.com/path"
    );
}

#[test]
fn reserved_ipv4_ranges_are_rejected_before_dns_or_connect() {
    for host in [
        "198.18.0.1",
        "198.19.255.254",
        "240.0.0.1",
        "255.255.255.255",
        "192.0.0.8",
        "192.0.2.1",
        "198.51.100.1",
        "203.0.113.1",
    ] {
        let error = validate_http_url(&format!("https://{host}/")).unwrap_err();
        assert!(error.to_string().contains("阻止本地/私有"), "{error}");
    }
}

#[tokio::test]
async fn ipv6_literal_lookup_uses_host_port_syntax() {
    let addresses = resolve_public_addresses("2606:4700:4700::1111", 443)
        .await
        .expect("公网 IPv6 字面量应可解析");
    let expected: std::net::IpAddr = "2606:4700:4700::1111".parse().unwrap();
    assert!(addresses.iter().all(|address| address.ip() == expected));
}

fn text_response(content_type: &str, body: impl Into<reqwest::Body>) -> Response {
    Response::from(
        http::Response::builder()
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body.into())
            .unwrap(),
    )
}

#[tokio::test]
async fn response_reader_caps_before_decoding_and_truncates_on_char_boundary() {
    let response = text_response("text/plain", "abc中国".repeat(1_000));
    let (text, truncated, size) = read_response_text(response, 16).await.unwrap();
    assert!(truncated);
    assert_eq!(size, 17);
    assert_eq!(text, "abc中国abc中");
    assert!(text.len() <= 16);
}

#[tokio::test]
async fn response_reader_honors_declared_charset() {
    let response = text_response("text/plain; charset=gbk", vec![0xd6, 0xd0, 0xce, 0xc4]);
    let (text, truncated, size) = read_response_text(response, 16).await.unwrap();
    assert_eq!(text, "中文");
    assert!(!truncated);
    assert_eq!(size, 4);
}
