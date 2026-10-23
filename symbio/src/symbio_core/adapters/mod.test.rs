//! `adapters` 单测 —— 四层预算表与令牌闸门（[plan/01 §10](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")、
//! [plan/05 §3.3](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")）。
//!
//! 编译期负例（低档令牌传给 `generate`）无法在常规测试里表达——它由**签名**承载：
//! `generate` 只收 `&FullModel`，给 `RuleOnly` 编译不过（参照
//! `docs/003-design/verify/latency_gate.rs` 的 `should_not_compile` feature）。
//! 这里守的是运行时可测的半场：预算表与档位语义。
//!
//! [D-096](../../plugins/model/README.md#d-096-快速档裁-ii-一次受限模型调用classifyonly-令牌闸门显式退役不得生成的强制力搬到返回类型与工具集判据上 "symbio/src/plugins/model/README") 退役了快速档令牌
//! （`ClassifyOnly` / `issue_fast` / `CanClassify` / `CanGenerate`）：那只原本守
//! 「分类端口」的令牌，端口在生产里从未存在。档位语义由**预算值**承载
//! （严格递增的四档）；`may_call_model` / `may_generate` 两个孤儿谓词无生产读方，
//! 随后一并删除。

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

/// 令牌是**零大小**的：凭据不该带载荷，带了就意味着「凭据里还能塞别的意思」。
///
/// 这条替掉了 D-096 退役的 `full_model_token_satisfies_classify_and_generate`
/// （那只 trait 层级随快速档令牌一起收掉）。判的是同一件事的另一半：闸门的开销
/// 为零、且加字段会让这条红——「顺手在令牌里塞个字段」是本仓最容易被放过的走形。
#[test]
fn tokens_are_zero_sized() {
    assert_eq!(std::mem::size_of::<RuleOnly>(), 0);
    assert_eq!(std::mem::size_of::<FullModel>(), 0);
    assert_eq!(std::mem::size_of::<TokenIssuer>(), 0);
}

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
