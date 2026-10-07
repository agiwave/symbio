//! `symbio/src/symbio_core/capability.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// serde 往返：声明字段序列化进 tools/list JSON；未声明字段不出现且旧 JSON 兼容
#[test]
fn context_retention_serde_roundtrip() {
    let meta = CapabilityMeta {
        name: "todo_write".into(),
        description: "d".into(),
        input_schema: serde_json::json!({"type": "object"}),
        context_retention: Some(CapabilityToolContextRetention::LastOnly),
        ..Default::default()
    };
    let v = serde_json::to_value(&meta).unwrap();
    assert_eq!(v["context_retention"], serde_json::json!("last_only"));
    let back: CapabilityMeta = serde_json::from_value(v).unwrap();
    assert_eq!(
        back.context_retention,
        Some(CapabilityToolContextRetention::LastOnly)
    );

    // 旧 JSON（无该字段）反序列化兼容，且序列化时不输出空字段
    let old: CapabilityMeta = serde_json::from_value(serde_json::json!({
        "name": "x", "description": "d", "parameters": {}
    }))
    .unwrap();
    assert_eq!(old.context_retention, None);
    let ser = serde_json::to_value(&old).unwrap();
    assert!(ser.get("context_retention").is_none());
}

/// keep_count：All → u32::MAX，LastN(n) → n.max(1)，LastOnly → 1
#[test]
fn keep_count_semantics() {
    assert_eq!(CapabilityToolContextRetention::All.keep_count(), u32::MAX);
    assert_eq!(CapabilityToolContextRetention::LastN(0).keep_count(), 1);
    assert_eq!(CapabilityToolContextRetention::LastN(3).keep_count(), 3);
    assert_eq!(CapabilityToolContextRetention::LastOnly.keep_count(), 1);
}

/// 未声明风险 ⇒ 生效档是 `Medium`（不是 `Low`）
///
/// 这条是**审批闸门的安全前提**：默认档决定了那些没写 `risk` 的工具（第三方
/// 插件、MCP 工具、新加的工具）在阈值比较里落在哪一档。曾一度把 `#[default]`
/// 标在 `Low` 上，于是未声明 = 免审批 —— 比显式声明 Low 还要宽，
/// 「声明」这个动作因此失去约束力。
#[test]
fn undeclared_risk_falls_back_to_medium() {
    let meta = CapabilityMeta {
        name: "third_party_tool".into(),
        description: "d".into(),
        input_schema: serde_json::json!({ "type": "object" }),
        ..Default::default()
    };
    assert_eq!(meta.risk, None, "本例的前提是「未声明」");
    assert_eq!(meta.effective_risk(), CapabilityRiskLevel::Medium);
}

/// 声明了就按声明的档，且 `with_risk` 是构造入口
#[test]
fn declared_risk_wins_over_the_default() {
    for level in [
        CapabilityRiskLevel::Low,
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::High,
    ] {
        let meta = CapabilityMeta::default().with_risk(level);
        assert_eq!(meta.effective_risk(), level);
    }
}

/// 排序即档位序（Low < Medium < High）——闸门靠 `tool_risk > threshold` 判审批
#[test]
fn risk_levels_are_ordered_low_to_high() {
    assert!(CapabilityRiskLevel::Low < CapabilityRiskLevel::Medium);
    assert!(CapabilityRiskLevel::Medium < CapabilityRiskLevel::High);
}

/// serde 形状：snake_case；未声明的 `risk` 不出现在 JSON 里（旧 JSON 仍可解析）
#[test]
fn risk_serde_shape() {
    let meta = CapabilityMeta::default().with_risk(CapabilityRiskLevel::High);
    assert_eq!(
        serde_json::to_value(&meta).unwrap()["risk"],
        serde_json::json!("high")
    );

    let undeclared = CapabilityMeta::default();
    assert!(serde_json::to_value(&undeclared)
        .unwrap()
        .get("risk")
        .is_none());
}
