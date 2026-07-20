//! prompt_budget.rs 单元测试
//!
//! 对应源文件: `prompt_budget.rs`

use super::*;

#[test]
fn test_estimate_tokens_cjk() {
    // 100 个中文字符 → 100 tokens
    let text = "你好世界".repeat(25); // 100 字
    assert_eq!(estimate_tokens(&text), 100);
}

#[test]
fn test_estimate_tokens_ascii() {
    // 400 个 ASCII 字符 → 100 tokens
    let text = "a".repeat(400);
    assert_eq!(estimate_tokens(&text), 100);
}

#[test]
fn test_estimate_tokens_mixed() {
    // 50 CJK + 200 ASCII = 50 + 50 = 100
    let cjk = "中".repeat(50);
    let ascii = "a".repeat(200);
    let text = format!("{}{}", cjk, ascii);
    assert_eq!(estimate_tokens(&text), 100);
}

#[test]
fn test_estimate_tokens_empty() {
    assert_eq!(estimate_tokens(""), 0);
}

#[test]
fn test_estimate_tokens_punctuation() {
    // 全角标点应被算作 CJK
    let text = "，。、！？".repeat(10); // 50 个全角符号（5 字符 × 10）
    assert_eq!(estimate_tokens(&text), 50);
}

#[test]
fn test_prompt_budget_default() {
    let b = PromptBudget::default();
    assert_eq!(b.total, 3500);
    assert_eq!(b.overhead, 500);
    assert_eq!(b.available_for_cus(), 3000);
}

#[test]
fn test_prompt_budget_new_overhead_clamped() {
    // overhead ≥ total 时被截断
    let b = PromptBudget::new(100, 200);
    assert_eq!(b.total, 100);
    assert_eq!(b.overhead, 99);
    assert_eq!(b.available_for_cus(), 1);
}

#[test]
fn test_prompt_budget_available() {
    let b = PromptBudget::new(1000, 200);
    assert_eq!(b.available_for_cus(), 800);
}

#[test]
fn test_budget_usage_remaining() {
    let mut u = BudgetUsage::new(1000);
    u.add_section("identity", 50);
    u.add_section("rules", 200);
    assert_eq!(u.used, 250);
    assert_eq!(u.remaining(), 750);
    assert!((u.usage_ratio() - 0.25).abs() < 1e-9);
}

#[test]
fn test_compute_cu_score_priority_dominance() {
    // priority 小的 CU 应得到更高评分（token 相同）
    let high = compute_cu_score("a", 1, 0.5, 0.0, 100);
    let low = compute_cu_score("b", 99, 0.5, 0.0, 100);
    assert!(high.score > low.score);
}

#[test]
fn test_compute_cu_score_belief_influence() {
    // meta_belief 高的 CU 应得到更高评分
    let trusted = compute_cu_score("a", 50, 0.9, 0.0, 100);
    let untrusted = compute_cu_score("b", 50, 0.1, 0.0, 100);
    assert!(trusted.score > untrusted.score);
}

#[test]
fn test_compute_cu_score_relevance_boost() {
    // 相关性高的 CU 应得到更高评分
    let relevant = compute_cu_score("a", 50, 0.5, 0.8, 100);
    let irrelevant = compute_cu_score("b", 50, 0.5, 0.0, 100);
    assert!(relevant.score > irrelevant.score);
}

#[test]
fn test_compute_cu_score_inputs_clamped() {
    // 异常输入应被 clamp 到合法范围
    let s = compute_cu_score("a", 50, 5.0, -1.0, 100);
    assert_eq!(s.meta_belief, 5.0); // 原值保留在 CuScore
                                    // score 中应使用 clamp 后的值
    assert!(s.score > 0.0);
}

#[test]
fn test_compute_cu_score_density_punishes_verbosity() {
    // 价值维度完全相同，token 少的 CU 密度更高（score 更高）
    let concise = compute_cu_score("a", 10, 0.5, 0.0, 50); // 1× TOKEN_NORM_UNIT
    let verbose = compute_cu_score("b", 10, 0.5, 0.0, 250); // 5× TOKEN_NORM_UNIT
    assert_eq!(
        concise.value, verbose.value,
        "价值维度相同则 value 必须相等"
    );
    assert!(
        concise.score > verbose.score,
        "短 CU 密度应更高：concise={} verbose={}",
        concise.score,
        verbose.score
    );
    // token_factor：50 token → 1.0；250 token → 5.0；密度比 5:1
    let ratio = verbose.score / concise.score;
    assert!(
        (ratio - 0.2).abs() < 0.01,
        "密度应反比于 token：实际比 {}",
        ratio
    );
}

#[test]
fn test_compute_cu_score_small_token_not_inflated() {
    // token < TOKEN_NORM_UNIT 时不应被放大（token_factor = max(1, ...) = 1）
    let tiny = compute_cu_score("a", 10, 0.5, 0.0, 1);
    let norm = compute_cu_score("b", 10, 0.5, 0.0, 50);
    assert_eq!(
        tiny.score, norm.score,
        "≤ TOKEN_NORM_UNIT 时密度 = 原始价值"
    );
    assert_eq!(tiny.value, tiny.score);
}

#[test]
fn test_is_cjk_basic() {
    assert!(is_cjk('中'));
    assert!(is_cjk('国'));
    assert!(is_cjk('，'));
    assert!(!is_cjk('a'));
    assert!(!is_cjk('1'));
    assert!(!is_cjk(' '));
}
