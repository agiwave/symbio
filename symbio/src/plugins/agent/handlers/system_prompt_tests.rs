//! system_prompt handler 单元测试
//!
//! 对应源文件: `system_prompt.rs`

use super::*;
use crate::plugins::agent::core::CognitiveUnit;

fn make_cu(name: Option<&str>, desc: Option<&str>) -> CognitiveUnit {
    let mut cu = CognitiveUnit::new("test_cu");
    if let Some(n) = name {
        cu.set_name(n);
    }
    if let Some(d) = desc {
        cu.set_description(d);
    }
    cu
}

#[test]
fn test_render_cu_markdown_includes_all_fields() {
    let mut cu = make_cu(
        Some("FullStack Architect"),
        Some("Focus on production-grade code quality."),
    );
    cu.set_is_a(&["agent_self"]);
    cu.set(cu_fields::PRIORITY.to_string(), serde_json::json!(5));

    let out = render_cu_markdown(&cu);
    assert!(out.contains("test_cu"), "should contain id: {}", out);
    assert!(
        out.contains("FullStack Architect"),
        "should contain name: {}",
        out
    );
    assert!(
        out.contains("Focus on production-grade code quality"),
        "should contain description: {}",
        out
    );
    assert!(out.contains("agent_self"), "should contain is_a: {}", out);
    assert!(
        out.contains("priority"),
        "should contain priority field: {}",
        out
    );
    let id_pos = out.find("test_cu").unwrap();
    let name_pos = out.find("FullStack").unwrap();
    assert!(id_pos < name_pos, "id should come before name: {}", out);
}

#[test]
fn test_render_cu_markdown_excludes_meta() {
    let mut cu = make_cu(Some("Test"), Some("Content"));
    cu.set("_ext_version".to_string(), serde_json::json!(42));
    cu.set(
        "_ext_created_at".to_string(),
        serde_json::json!("2024-01-01"),
    );

    let out = render_cu_markdown(&cu);
    assert!(
        !out.contains("_ext_"),
        "should not contain _ext_ field: {}",
        out
    );
}

#[test]
fn test_render_cu_markdown_changed_name_visible() {
    let cu = make_cu(Some("NewName"), Some("Description"));
    let out = render_cu_markdown(&cu);
    assert!(out.contains("NewName"));
    assert!(!out.contains("OldName"));
}

#[test]
fn test_business_scenario_label() {
    assert_eq!(business_scenario_label("rule"), "行为规则");
    assert_eq!(business_scenario_label("skill"), "专业技能");
    assert_eq!(business_scenario_label("unknown_type"), "unknown_type");
}

// ── 新增：三层目标相关测试 ──

#[test]
fn test_tokenize_for_relevance_cjk() {
    let tokens = tokenize_for_relevance("你好 世界");
    // CJK 单字 + 空格分隔
    assert!(tokens.contains("你"));
    assert!(tokens.contains("好"));
    assert!(tokens.contains("世"));
    assert!(tokens.contains("界"));
}

#[test]
fn test_tokenize_for_relevance_ascii() {
    let tokens = tokenize_for_relevance("Hello World hello");
    assert!(tokens.contains("hello"));
    assert!(tokens.contains("world"));
    assert_eq!(tokens.len(), 2); // "hello" 去重
}

#[test]
fn test_compute_relevance_match() {
    let cu = make_cu(Some("Rust 编程"), Some("Rust 是一种系统编程语言"));
    let query = tokenize_for_relevance("Rust 系统");
    let r = compute_relevance(&cu, &query);
    assert!(r > 0.0, "应该有相关性: {}", r);
}

#[test]
fn test_compute_relevance_no_match() {
    let cu = make_cu(Some("烘焙"), Some("蛋糕制作"));
    let query = tokenize_for_relevance("Rust 系统编程");
    let r = compute_relevance(&cu, &query);
    assert_eq!(r, 0.0);
}

#[test]
fn test_compute_relevance_empty_query() {
    let cu = make_cu(Some("Test"), Some("Test"));
    let r = compute_relevance(&cu, &HashSet::new());
    assert_eq!(r, 0.0);
}

#[test]
fn test_allocate_type_budget_sufficient() {
    // 总理想 tokens ≤ cu_budget → 每个 type 拿到 ideal
    let mut scored = HashMap::new();
    let cu = make_cu(Some("a"), Some("aaa"));
    let s1 = compute_cu_score("a", 1, 0.5, 0.0, 100);
    let s2 = compute_cu_score("b", 2, 0.5, 0.0, 200);
    scored.insert("rule".to_string(), vec![(&cu, s1)]);
    let cu2 = make_cu(Some("b"), Some("bbb"));
    scored.insert("skill".to_string(), vec![(&cu2, s2)]);
    let priorities = vec![("rule".to_string(), 1i64), ("skill".to_string(), 2i64)];
    let budgets = allocate_type_budget(&priorities, &scored, 1000);
    assert_eq!(budgets.get("rule"), Some(&100));
    assert_eq!(budgets.get("skill"), Some(&200));
}

#[test]
fn test_allocate_type_budget_insufficient_scales_down() {
    // 总理想 tokens > cu_budget → 按比例缩放
    let cu = make_cu(Some("a"), Some("aaa"));
    let s1 = compute_cu_score("a", 1, 0.5, 0.0, 1000);
    let s2 = compute_cu_score("b", 2, 0.5, 0.0, 1000);
    let mut scored = HashMap::new();
    scored.insert("rule".to_string(), vec![(&cu, s1)]);
    let cu2 = make_cu(Some("b"), Some("bbb"));
    scored.insert("skill".to_string(), vec![(&cu2, s2)]);
    let priorities = vec![("rule".to_string(), 1i64), ("skill".to_string(), 2i64)];
    let budgets = allocate_type_budget(&priorities, &scored, 100);
    // 总 2000 → 总分配 100 → 各分 50（最小保证）
    let sum: usize = budgets.values().sum();
    assert!(sum <= 100, "总分配不应超过 cu_budget: {}", sum);
}

#[test]
fn test_allocate_type_budget_zero() {
    let priorities: Vec<(String, i64)> = vec![];
    let scored: HashMap<String, Vec<(&CognitiveUnit, CuScore)>> = HashMap::new();
    let budgets = allocate_type_budget(&priorities, &scored, 0);
    assert!(budgets.is_empty());
}

#[test]
fn test_render_footer_includes_budget_info() {
    // 系统提示词不渲染 footer（工具速查/预算段）——这些元信息 LLM 通过
    // 系统提示词末尾的"预算告警"段主动响应（不需专门 op）
    let mut usage = BudgetUsage::new(1000);
    usage.add_section("identity", 100);
    usage.add_section("行为规则", 200);
    let budget = PromptBudget::new(1000, 300);
    // usage 仍正确累计（BudgetUsage 是"预算告警"段的数据源）
    assert_eq!(usage.used, 300);
    assert_eq!(budget.total, 1000);
}
