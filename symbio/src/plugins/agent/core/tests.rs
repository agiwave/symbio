//! core/mod.rs 单元测试
//!
//! 对应源文件: `mod.rs`
//!
//! 主要内容：公开 API 烟测，保证顶层 reexport 全部可用。
//! 不通过 `#[allow(unused_imports)]` 抑制警告，而是**真实引用**每个 reexport
//! 类型一次——任何被误删/重命名的项都会让本块编译失败，从而暴露问题。

use super::*;

#[test]
fn reexports_are_constructible() {
    // 引用每个 pub reexport——保证不被 unused_imports 警告且真能引用到目标类型
    let _: Option<AgentConfig> = None;
    let _: Option<CognitionThresholds> = None;
    let _: Option<StorageBackendType> = None;
    let _: Option<StorageFormat> = None;
    let _: Option<Box<dyn EmbeddingService>> = None;
    let _: Option<AgentError> = None;
    // AgentResult 是 type alias，不做 None 测试
    let _: Option<Box<dyn AgentStore>> = None;
    let _: Option<FilterExpr> = None;
    let _: Option<PageRequest> = None;
    let _: Option<PageResult> = None;
    let _: Option<StoreError> = None;
    let _: Option<CognitionContext> = None;
    let _: Option<CognitiveUnit> = None;
    let _: Option<OperationResult> = None;
    // 三层目标基础设施 reexport
    let _: Option<PromptBudget> = None;
    let _: Option<BudgetUsage> = None;
    let _: Option<CuScore> = None;

    // 函数 reexport
    let _ = cu_from_json(serde_json::Value::Null);
    let _: u64 = now_secs();
    let _ = truncate_chars("", 0);
    let _cu = CognitiveUnit::new("test");
    let _ = unit_with_id(&_cu);
    let _ = evaluate_filter(&_cu, &FilterExpr::match_all());
    let _ = cosine_similarity(&[], &[]);
    // 三层目标函数 reexport
    let _: usize = estimate_tokens("");
    let _ = compute_cu_score("a", 1, 0.5, 0.0, 100);
}
