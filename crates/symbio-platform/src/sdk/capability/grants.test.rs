//! `ApprovalGrants` 的判据：读侧逐条容错、上限只抬不降、线格式就是一张映射。

use super::*;
use crate::kernel::plugin::PluginSimpleRequest;

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 清单按**能力名**记，值只认三个档位词；认不出的那一行被丢掉（fail-closed）。
#[test]
fn parse_keeps_only_known_levels_and_non_empty_tools() {
    let g = ApprovalGrants::parse(
        r#"{"shell":"high","vdfs_read":"low","mcp.srv.tool":"medium","bad":"critical","":"high"," ":"low"}"#,
    );
    assert_eq!(g.ceiling("shell"), Some(CapabilityRiskLevel::High));
    assert_eq!(g.ceiling("vdfs_read"), Some(CapabilityRiskLevel::Low));
    assert_eq!(g.ceiling("mcp.srv.tool"), Some(CapabilityRiskLevel::Medium));
    assert_eq!(g.ceiling("bad"), None, "未知档位不能猜成某一档");
    assert_eq!(g.ceiling(""), None);
    assert_eq!(g.ceiling(" "), None);
}

/// 非对象 / 非法 JSON / 空串 ⇒ 空清单——没有授权就没有免问。
#[test]
fn parse_fails_closed_on_broken_input() {
    for raw in ["", "not json", "[]", "null", r#""high""#, "42"] {
        assert_eq!(
            ApprovalGrants::parse(raw),
            ApprovalGrants::default(),
            "{raw:?}"
        );
    }
}

/// 一份 `PLUGIN.yml` 里的笔误只丢那一行，不拖垮整份配置。
#[test]
fn deserialize_drops_one_bad_entry_instead_of_failing() {
    let g: ApprovalGrants =
        serde_json::from_str(r#"{"shell":"high","oops":"HIGH","vdfs_read":7}"#).unwrap();
    assert_eq!(g.ceiling("shell"), Some(CapabilityRiskLevel::High));
    assert_eq!(g.ceiling("oops"), None);
    assert_eq!(g.ceiling("vdfs_read"), None);
}

/// 上限**只抬不降**：一次更低档的批准不改写已有授权。
#[test]
fn raise_only_goes_up() {
    let mut g = ApprovalGrants::default();
    assert_eq!(
        g.raise("shell", CapabilityRiskLevel::High),
        CapabilityRiskLevel::High
    );
    assert_eq!(
        g.raise("shell", CapabilityRiskLevel::Low),
        CapabilityRiskLevel::High,
        "降档等于顺手收紧用户已有的授权"
    );
    assert_eq!(g.ceiling("shell"), Some(CapabilityRiskLevel::High));
    assert_eq!(
        g.raise("shell", CapabilityRiskLevel::Medium),
        CapabilityRiskLevel::High
    );
}

/// 线格式往返：序列化产出的就是配置文档里的那张映射。
#[test]
fn wire_format_round_trips_as_a_plain_map() {
    let mut g = ApprovalGrants::default();
    g.raise("shell", CapabilityRiskLevel::High);
    let json = serde_json::to_string(&g).expect("映射必然可序列化");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value, serde_json::json!({ "shell": "high" }));
    assert_eq!(ApprovalGrants::parse(&json), g);
}

/// `null` 与缺席同解（表单交回空槽位时不该让整份配置反序列化失败）。
#[test]
fn null_deserializes_to_empty() {
    let g: ApprovalGrants = serde_json::from_str("null").unwrap();
    assert_eq!(g, ApprovalGrants::default());
}

#[test]
fn one_time_grant_is_bound_and_ignores_model_approved_field() {
    let c = ctx();
    let args = serde_json::json!({"command":"git status","approved":true});
    let grant = ApprovalGrant::issue(
        "call-1",
        "shell",
        &args,
        Some("session-1".into()),
        7,
        Duration::from_secs(60),
    );
    grant.attach(&c);
    assert!(ApprovalGrant::take(
        c.as_ref(),
        "call-1",
        "shell",
        &serde_json::json!({"command":"git status"}),
        Some("session-1"),
        7,
    ));
    assert!(!ApprovalGrant::take(
        c.as_ref(),
        "call-1",
        "shell",
        &serde_json::json!({"command":"git status"}),
        Some("session-1"),
        7,
    ));
}

#[test]
fn grant_rejects_changed_call_tool_args_session_version_and_expiry() {
    let args = serde_json::json!({"command":"git status"});
    for (index, (call, tool, changed, session, version, ttl)) in [
        ("other", "shell", args.clone(), "s", 1, 60),
        ("c", "other", args.clone(), "s", 1, 60),
        (
            "c",
            "shell",
            serde_json::json!({"command":"rm -rf /"}),
            "s",
            1,
            60,
        ),
        ("c", "shell", args.clone(), "other", 1, 60),
        ("c", "shell", args.clone(), "s", 2, 60),
        ("c", "shell", args.clone(), "s", 1, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let c = ctx();
        ApprovalGrant::issue(
            "c",
            "shell",
            &args,
            Some("s".into()),
            1,
            Duration::from_secs(ttl),
        )
        .attach(&c);
        assert!(
            !ApprovalGrant::take(c.as_ref(), call, tool, &changed, Some(session), version,),
            "case {index} must reject"
        );
    }
}
