//! agent chat handler (ContextBuilder) 单元测试
//!
//! 对应源文件: `chat.rs`

use super::*;
use crate::plugins::agent::store::build_in_memory_store;

#[tokio::test]
async fn context_builder_empty_store_returns_empty() {
    let store = build_in_memory_store();
    let cb = ContextBuilder::default();
    let result = cb.build(store.as_ref(), "hello world", None).await;
    // 空 store → 只有 temporal context（不为空）
    assert!(result.contains("<context>"), "应包含时间上下文");
}

#[tokio::test]
async fn context_builder_short_query_skips_memory() {
    let store = build_in_memory_store();
    let cb = ContextBuilder::default();
    let result = cb.build(store.as_ref(), "hi", None).await;
    // 短查询（<4 字符）不搜索记忆
    assert!(
        !result.contains("<active_memory>"),
        "短查询不应触发记忆搜索"
    );
}

#[tokio::test]
async fn context_builder_filters_identity_and_rules() {
    use crate::plugins::agent::core::CognitiveUnit;
    use crate::plugins::agent::store::build_test_scaffold;

    let store = build_test_scaffold().await;

    // 插入一个 identity CU
    let mut identity = CognitiveUnit::default();
    identity.set_id("identity");
    identity.set_name("identity");
    identity.set("content", serde_json::json!("我是 MODEL 助手"));
    identity.set("level", serde_json::json!("sys"));
    store.insert(&identity).await.unwrap();

    // 插入一个 rule CU
    let mut rule = CognitiveUnit::default();
    rule.set_id("rule_001");
    rule.set_name("rule_001");
    rule.set("content", serde_json::json!("代码规范"));
    rule.add_relation("is_a", "rule");
    store.insert(&rule).await.unwrap();

    // 插入一个普通 fact CU
    let mut fact = CognitiveUnit::default();
    fact.set_id("fact_001");
    fact.set_name("fact_001");
    fact.set("content", serde_json::json!("Rust 是系统编程语言"));
    fact.add_relation("is_a", "fact");
    store.insert(&fact).await.unwrap();

    let cb = ContextBuilder::default();
    let result = cb
        .build_active_memory(store.as_ref(), "Rust 编程语言")
        .await;
    // identity 和 rule 应被过滤
    assert!(!result.contains("identity"), "identity 应被过滤");
    assert!(!result.contains("代码规范"), "rule 应被过滤");
}

#[tokio::test]
async fn context_builder_task_context_includes_strategies() {
    use crate::plugins::agent::core::CognitiveUnit;
    use crate::plugins::agent::store::build_test_scaffold;

    let store = build_test_scaffold().await;

    // 插入 strategy
    let mut strategy = CognitiveUnit::default();
    strategy.set_id("strat_001");
    strategy.set_name("分治策略");
    strategy.set("content", serde_json::json!("将复杂问题分解为子问题"));
    strategy.add_relation("is_a", "strategy");
    store.insert(&strategy).await.unwrap();

    // 插入 skill
    let mut skill = CognitiveUnit::default();
    skill.set_id("skill_001");
    skill.set_name("代码审查");
    skill.set("content", serde_json::json!("系统性检查代码质量"));
    skill.add_relation("is_a", "skill");
    store.insert(&skill).await.unwrap();

    let cb = ContextBuilder::default();
    let result = cb.build_task_context(store.as_ref()).await;
    assert!(
        result.contains("<task_context>"),
        "应包含 task_context 标签"
    );
    assert!(result.contains("分治策略"), "应包含 strategy 名称");
    assert!(result.contains("代码审查"), "应包含 skill 名称");
}

#[tokio::test]
async fn build_temporal_context_contains_time() {
    let result = build_temporal_context(Some("/workspace"));
    assert!(result.contains("<context>"), "应包含 context 标签");
    assert!(result.contains("工作区: /workspace"), "应包含工作区路径");
    assert!(result.contains("星期"), "应包含星期信息");
}

#[tokio::test]
async fn build_temporal_context_without_workdir() {
    let result = build_temporal_context(None);
    assert!(result.contains("<context>"), "应包含 context 标签");
    assert!(!result.contains("工作区"), "无 workdir 时不应包含工作区");
}

/// I-059 验证：临时上下文块开头有元说明
#[tokio::test]
async fn context_builder_includes_meta_header() {
    let store = build_in_memory_store();
    let cb = ContextBuilder::default();
    let result = cb.build(store.as_ref(), "hello world", None).await;
    assert!(
        result.starts_with("## 临时上下文"),
        "临时上下文应包含元说明段，便于 LLM 理解"
    );
    // 用 `` 包裹的标签名（避免与实际 XML 标签冲突）
    assert!(
        result.contains("`active_memory`"),
        "应说明 active_memory 标签"
    );
    assert!(result.contains("`context`"), "应说明 context 标签");
    assert!(
        result.contains("`task_context`"),
        "应说明 task_context 标签"
    );
}
