//! `adapters` 单测 —— 四层预算表与令牌层级（[plan/01 §10](../../../../docs/plan/01-核心架构.md)、
//! [plan/05 §3.3](../../../../docs/plan/05-模块架构.md)）。
//!
//! 编译期负例（`RuleOnly` 不满足 `CanClassify` / `ClassifyOnly` 不满足 `CanGenerate`）
//! 无法在常规测试里表达——它们由**签名**承载：`generate` 只收 `&FullModel`，
//! 给任何低档令牌都编译不过（参照 `docs/plan/verify/latency_gate.rs` 的
//! `should_not_compile` feature）。这里守的是运行时可测的半场。

use super::*;

/// 验收（01 §10）：四层预算严格递增，取值与 §8 参数表一致。
#[test]
fn four_tier_budgets_are_strictly_increasing() {
    let tiers = [
        LatencyTier::Reflex,
        LatencyTier::Fast,
        LatencyTier::Deep,
        LatencyTier::Autonomic,
    ];
    for pair in tiers.windows(2) {
        assert!(
            pair[0].budget_ms() < pair[1].budget_ms(),
            "{:?}({}) 必须严格小于 {:?}({})",
            pair[0],
            pair[0].budget_ms(),
            pair[1],
            pair[1].budget_ms()
        );
    }
    assert_eq!(LatencyTier::Reflex.budget_ms(), 80);
    assert_eq!(LatencyTier::Fast.budget_ms(), 300);
    assert_eq!(LatencyTier::Deep.budget_ms(), 60_000);
    assert_eq!(LatencyTier::Autonomic.budget_ms(), 86_400_000);
}

/// 验收（01 §10）：反射层禁止模型；快速档只分类；生成仅深度/自主档。
#[test]
fn tier_capabilities_match_the_budget_table() {
    assert!(!LatencyTier::Reflex.may_call_model(), "反射层禁模型");
    assert!(LatencyTier::Fast.may_call_model());
    assert!(!LatencyTier::Fast.may_generate(), "快速档只分类不生成");
    assert!(LatencyTier::Deep.may_generate());
    assert!(LatencyTier::Autonomic.may_generate());
}

/// 高档能做低档的事（trait 层级）：`FullModel` 同时满足 `CanClassify` 与
/// `CanGenerate`，`ClassifyOnly` 只满足 `CanClassify`（负例由签名承载，见文件头）。
#[test]
fn full_model_token_satisfies_classify_and_generate() {
    fn assert_can_classify<T: CanClassify>(_: &T) {}
    fn assert_can_generate<T: CanGenerate>(_: &T) {}
    let deep = TokenIssuer::issue_deep();
    assert_can_classify(&deep);
    assert_can_generate(&deep);
    let fast = TokenIssuer::issue_fast();
    assert_can_classify(&fast);
}

/// 一条 user 消息的请求包（结构化入参，ADR-048a）。
fn one(text: &str) -> Vec<PromptMessage> {
    vec![PromptMessage {
        role: "user".into(),
        text: text.into(),
        tool_call_id: None,
        tool: None,
        tool_calls: None,
    }]
}

/// 桩的两种演练形态：确定性成功 / 确定性失败。
#[tokio::test]
async fn stub_adapter_succeeds_and_fails_deterministically() {
    let ok = StubLlmAdapter::succeed("stub-model");
    let tok = TokenIssuer::issue_deep();
    let greeting = one("你好");
    let out = ok.generate(&tok, &greeting).await.expect("成功桩必答");
    assert!(out.contains("stub-model"), "{}", out);

    let bad = StubLlmAdapter::always_fail("模型不可用");
    let x = one("x");
    assert!(
        matches!(bad.generate(&tok, &x).await, Err(AdapterError::GenerationFailed(m)) if m.contains("不可用")),
        "失败桩必须确定性失败（演练兜底）"
    );
}
