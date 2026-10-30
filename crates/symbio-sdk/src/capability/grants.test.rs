//! `ApprovalGrants` 的判据：读侧逐条容错、上限只抬不降、线格式就是一张映射。

use super::*;

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

/// 线格式往返：`to_json` 产出的就是配置文档里的那张映射。
#[test]
fn wire_format_round_trips_as_a_plain_map() {
    let mut g = ApprovalGrants::default();
    g.raise("shell", CapabilityRiskLevel::High);
    let json = g.to_json();
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
