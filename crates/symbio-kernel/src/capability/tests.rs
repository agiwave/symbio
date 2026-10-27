//! `crates/symbio-core/src/capability/mod.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// serde 往返：声明字段序列化进 tools/list JSON；未声明字段不出现且旧 JSON 兼容
///
/// 载体取 `risk`：它是最早的「自声明枚举字段」，本条钉的是**这一类字段的线格式
/// 契约**（有值才出现、旧 JSON 缺键即缺省），不是某一个字段的语义。
#[test]
fn declared_enum_field_serde_roundtrip() {
    let meta = CapabilityMeta {
        name: "todo_write".into(),
        description: "d".into(),
        input_schema: serde_json::json!({"type": "object"}),
        risk: Some(CapabilityRiskLevel::High),
        ..Default::default()
    };
    let v = serde_json::to_value(&meta).unwrap();
    assert_eq!(v["risk"], serde_json::json!("high"));
    let back: CapabilityMeta = serde_json::from_value(v).unwrap();
    assert_eq!(back.risk, Some(CapabilityRiskLevel::High));

    // 旧 JSON（无该字段）反序列化兼容，且序列化时不输出空字段
    let old: CapabilityMeta = serde_json::from_value(serde_json::json!({
        "name": "x", "description": "d", "parameters": {}
    }))
    .unwrap();
    assert_eq!(old.risk, None);
    let ser = serde_json::to_value(&old).unwrap();
    assert!(ser.get("risk").is_none());
}

/// 行为名是宿主内部词汇：`declares` 按整词判，序列化时不进 tools/list JSON。
///
/// 这是 D-210 的线格式边界：行为名决定宿主闸门，不是模型看到的工具属性。
/// 进 wire 会把「哪些行为」变成对外协议，改名就要兼容两代客户端。
#[test]
fn behaviors_are_host_vocabulary_not_wire() {
    let meta = CapabilityMeta {
        name: "x".into(),
        behaviors: &["assign_work", "task_table"],
        ..Default::default()
    };
    assert!(meta.declares("assign_work"));
    assert!(meta.declares("task_table"));
    assert!(!meta.declares("assign"), "按整词判，不按前缀");

    let v = serde_json::to_value(&meta).unwrap();
    assert!(v.get("behaviors").is_none(), "行为名不应进入 wire");
    let back: CapabilityMeta = serde_json::from_value(v).unwrap();
    assert!(back.behaviors.is_empty(), "旧 JSON 缺键 ⇒ 空集");
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

/// `from_name` 与 serde 的线格式**同一套字面量**——两处不一致就会分叉。
///
/// 这个仓有两个方向的风险等级字符串：serde（`CapabilityMeta.risk` 进出 tools/list
/// JSON）与 `ctx[RISK_LEVEL]`（会话阈值，人写在会话元数据里）。若 `from_name` 认的词
/// 与 serde 产出的词不同，同一档位两边读出不同结果，而**两边都不会报错**。
#[test]
fn from_name_agrees_with_serde_wire_format() {
    for level in [
        CapabilityRiskLevel::Low,
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::High,
    ] {
        let wire = serde_json::to_value(level).unwrap();
        let name = wire.as_str().unwrap();
        assert_eq!(
            CapabilityRiskLevel::from_name(name),
            level,
            "serde 产出 {name:?}，from_name 却读成别的档位"
        );
    }
}

/// 未识别的值 ⇒ `Medium`（宁可多问一次），不 panic、不放宽。
///
/// `"critical"` 是刻意选的：它看起来像「比 high 更高」，而本档位没有这一档。
/// 若哪天有人往会话元数据里写它，正确行为是**退回默认档**（要审批），而不是当成
/// `Low` 放行、也不是崩在一次无关的会话启动上。
#[test]
fn from_name_falls_back_to_medium_on_unknown() {
    for raw in ["", "medium", "critical", "HIGH", "low ", "0"] {
        assert_eq!(
            CapabilityRiskLevel::from_name(raw),
            CapabilityRiskLevel::Medium,
            "{raw:?} 应退回默认档"
        );
    }
}

/// 严格解析只认线格式的三个词——写授权清单时一个笔误必须被**拒绝**，
/// 不能被静默折成 `Medium` 后写进「永久放行」。
#[test]
fn parse_is_strict_where_from_name_defaults() {
    for level in [
        CapabilityRiskLevel::Low,
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::High,
    ] {
        assert_eq!(CapabilityRiskLevel::parse(level.as_str()), Some(level));
        assert_eq!(
            level.as_str(),
            serde_json::to_value(level).unwrap().as_str().unwrap()
        );
    }
    for raw in ["", "critical", "HIGH", "low ", "0"] {
        assert_eq!(CapabilityRiskLevel::parse(raw), None, "{raw:?}");
        assert_eq!(
            CapabilityRiskLevel::from_name(raw),
            CapabilityRiskLevel::Medium
        );
    }
}

/// 三档审批的**同一条判据**：依据各就各位，优先级 approved > threshold > grant。
#[test]
fn approval_basis_covers_the_three_tiers_in_order() {
    use CapabilityRiskLevel::{High, Low, Medium};

    // 第 1 档：这一次已批准——哪怕远高于阈值与上限。
    assert_eq!(
        High.approval_basis(Medium, true, None),
        Some(ApprovalBasis::Approved)
    );
    // 第 2 档：不高于会话阈值。
    assert_eq!(
        Medium.approval_basis(Medium, false, None),
        Some(ApprovalBasis::Threshold)
    );
    assert_eq!(
        Low.approval_basis(Medium, false, None),
        Some(ApprovalBasis::Threshold)
    );
    // 第 3 档：高于阈值但落在该工具的永久上限内。
    assert_eq!(
        High.approval_basis(Medium, false, Some(High)),
        Some(ApprovalBasis::Grant)
    );
    // 依据优先级：同时满足阈值与上限时，记的是更宽的那条线（会话阈值）。
    assert_eq!(
        Medium.approval_basis(Medium, false, Some(High)),
        Some(ApprovalBasis::Threshold)
    );
    // 三档都不成立 ⇒ 需要审批。
    assert_eq!(High.approval_basis(Medium, false, Some(Medium)), None);
    assert_eq!(High.approval_basis(Medium, false, None), None);
}
