//! `dispatch::parallel` 的单元测试：**策略映射**本身（不跑工具）。

use super::*;
use crate::symbio_core::CapabilityMeta;

fn meta(name: &str, risk: Option<CapabilityRiskLevel>) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        risk,
        ..Default::default()
    }
}

/// 映射是唯一判定点：`Low` 并行，其余（含未声明）串行。
#[test]
fn only_low_risk_runs_in_parallel() {
    assert_eq!(
        policy_for_risk(CapabilityRiskLevel::Low),
        ExecPolicy::Parallel
    );
    assert_eq!(
        policy_for_risk(CapabilityRiskLevel::Medium),
        ExecPolicy::Serial
    );
    assert_eq!(
        policy_for_risk(CapabilityRiskLevel::High),
        ExecPolicy::Serial
    );
}

/// 交互模式整批串行——即使每条调用的风险档都允许并行。
#[test]
fn interactive_mode_forces_serial() {
    assert_eq!(
        policy_for_mode("interactive", ExecPolicy::Parallel),
        ExecPolicy::Serial
    );
    assert_eq!(
        policy_for_mode("auto", ExecPolicy::Parallel),
        ExecPolicy::Parallel
    );
}

/// 端口声明是**硬闸**：`concurrent=false` 压过风险档与模式；`true` 时逐字沿用
/// 既有规则。这是「并发集合是声明，不是执行体自觉」的最小判据。
#[test]
fn a_non_concurrent_port_forces_serial() {
    assert_eq!(
        policy_for_port(false, "auto", ExecPolicy::Parallel),
        ExecPolicy::Serial
    );
    assert_eq!(
        policy_for_port(true, "auto", ExecPolicy::Parallel),
        ExecPolicy::Parallel
    );
    assert_eq!(
        policy_for_port(true, "interactive", ExecPolicy::Parallel),
        ExecPolicy::Serial
    );
}

/// 两种名字形态都能查到：注册表名（`local/read`）与线上名（`local__read`）。
/// 只收一种的话，模型发来的名字永远查不到 ⇒ 全部落到 `Medium` ⇒ 并行从不生效，
/// 而这条静默失效没有任何东西会红。
#[test]
fn both_canonical_and_wire_names_resolve() {
    let risks = ToolRisks::of_metas(&[meta("local/read", Some(CapabilityRiskLevel::Low))]);
    assert_eq!(risks.policy_for_call("local__read"), ExecPolicy::Parallel);
    assert_eq!(risks.policy_for_call("local/read"), ExecPolicy::Parallel);
}

/// 未声明的工具按 `Medium` 兜底（与 `CapabilityMeta::effective_risk` 同一条纪律），
/// 于是它们串行——「没写风险」不等于「只读」。
#[test]
fn undeclared_or_unknown_tools_are_serial() {
    let risks = ToolRisks::of_metas(&[meta("local/read", None)]);
    assert_eq!(
        risks.effective_risk("local__read"),
        CapabilityRiskLevel::Medium
    );
    assert_eq!(
        risks.policy_for_call("never/registered"),
        ExecPolicy::Serial
    );
}
