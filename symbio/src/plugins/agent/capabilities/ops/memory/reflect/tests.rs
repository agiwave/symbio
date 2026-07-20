//! memory/reflect.rs 单元测试
//!
//! 对应源文件: `memory/reflect.rs`

use super::*;
use crate::plugins::agent::capabilities::ops::CognitionOp;
use crate::plugins::agent::core::FilterExpr;
use crate::plugins::agent::core::PageRequest;

/// 基本反思：带 insights，验证 experience CU 被创建
#[tokio::test]
async fn reflect_creates_insight_cus() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let op = ReflectOp;

    let result = op
        .execute(
            engine.clone(),
            &json!({
                "summary": "调试了 Rust 借用错误",
                "insights": [
                    {"type": "experience", "content": "生命周期标注要从调用处反推", "confidence": 0.85},
                    {"type": "fact", "content": "&str 是 fat pointer", "confidence": 0.9}
                ]
            }),
        )
        .await;

    assert!(result.success, "reflect 应成功: {:?}", result.error);
    let data = result.data.unwrap();
    assert_eq!(
        data["created_count"], 2,
        "应创建 2 条 insight（反思日志单独统计在 reflection_log_id）"
    );
    assert!(
        !data["reflection_log_id"].as_str().unwrap().is_empty(),
        "应有反思日志 id"
    );

    // 验证 experience CU 可被检索（至少含刚创建的 2 条 insight）
    let page = engine
        .query(&FilterExpr::is_a("experience"), &PageRequest::first(20))
        .await
        .unwrap();
    assert!(
        page.total >= 2,
        "应至少有 2 个 experience CU（2 insight），实际 {}",
        page.total
    );
}

/// 反思：带 rule_updates，验证新建规则
#[tokio::test]
async fn reflect_creates_new_rule() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let op = ReflectOp;

    let result = op
        .execute(
            engine.clone(),
            &json!({
                "summary": "发现工具调用优化",
                "rule_updates": [
                    {"pattern": "批量写文件前先检查目录", "action": "避免重复创建目录", "confidence": 0.9}
                ]
            }),
        )
        .await;

    assert!(result.success, "reflect 应成功: {:?}", result.error);
    // 验证 rule CU 被创建
    let page = engine
        .query(&FilterExpr::is_a("rule"), &PageRequest::first(20))
        .await
        .unwrap();
    let found = page.items.iter().any(|cu| {
        cu.description()
            .map(|d| d.contains("批量写文件前先检查目录"))
            .unwrap_or(false)
    });
    assert!(found, "应包含新建的 rule CU");
}

/// 反思：带 id 的 rule_update，验证局部更新
#[tokio::test]
async fn reflect_updates_existing_rule() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;

    // 先手动创建一个 rule CU
    let mut existing = CognitiveUnit::generate_id();
    let existing_id = existing.id().to_string();
    existing.set_name("旧规则");
    existing.set_description("旧描述");
    existing.add_type("rule");
    existing.set_confidence(0.5);
    existing.set_meta_belief(0.5);
    engine.upsert(&existing).await.unwrap();

    // 反思更新它
    let op = ReflectOp;
    let result = op
        .execute(
            engine.clone(),
            &json!({
                "summary": "修订规则",
                "rule_updates": [
                    {"id": existing_id, "pattern": "新条件", "action": "新动作", "confidence": 0.85}
                ]
            }),
        )
        .await;

    assert!(result.success, "reflect 应成功: {:?}", result.error);
    assert_eq!(result.data.as_ref().unwrap()["updated_count"], 1);

    // 验证 belief 被提升（+0.05）
    let updated = engine.get(&existing_id).await.unwrap().unwrap();
    let belief = updated.meta_belief();
    assert!(
        (belief - 0.55).abs() < 0.01,
        "belief 应从 0.5 提升到 0.55，实际 {}",
        belief
    );
    assert!(updated.description().unwrap().contains("新条件"));
}

/// 反思：缺少 summary 应报错
#[tokio::test]
async fn reflect_missing_summary_errors() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let op = ReflectOp;

    let result = op.execute(engine, &json!({"insights": []})).await;
    assert!(!result.success, "缺少 summary 应失败");
    assert!(result.error.unwrap().contains("summary"));
}

/// 反思：insight 缺少 content 应记录错误但不阻断其它项
#[tokio::test]
async fn reflect_invalid_insight_recorded_in_errors() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let op = ReflectOp;

    let result = op
        .execute(
            engine,
            &json!({
                "summary": "混合测试",
                "insights": [
                    {"content": "有效洞察"},
                    {"type": "fact"}  // 缺 content
                ]
            }),
        )
        .await;

    assert!(result.success, "整体应成功（部分失败记入 errors）");
    let data = result.data.unwrap();
    let errors = data["errors"].as_array().unwrap();
    assert!(!errors.is_empty(), "应记录 content 缺失错误");
    assert!(
        data["created_count"].as_u64().unwrap() >= 1,
        "有效的 insight 应被创建"
    );
}

/// 反思：验证反思日志（meta_reflection）被写入
#[tokio::test]
async fn reflect_writes_reflection_log() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let op = ReflectOp;

    let result = op
        .execute(engine.clone(), &json!({"summary": "测试反思日志写入"}))
        .await;

    assert!(result.success);
    let reflection_id = result.data.unwrap()["reflection_log_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!reflection_id.is_empty(), "应返回反思日志 id");

    let log = engine.get(&reflection_id).await.unwrap().unwrap();
    assert_eq!(
        log.get("reflection_kind").and_then(|v| v.as_str()),
        Some("meta_reflection")
    );
    // 反思日志 priority=10（候选池内但不是最高优先级，避免日志挤占预算）
    assert_eq!(log.get("priority").and_then(|v| v.as_i64()), Some(10));
}

/// 验证 op 已注册到 registry
#[test]
fn reflect_registered_in_registry() {
    let registry = crate::plugins::agent::capabilities::ops::get_registry();
    assert!(
        registry.get("memory.reflect").is_some(),
        "memory.reflect 应已注册"
    );
}

/// 验证 meta 的 schema 完整性
#[test]
fn reflect_meta_schema_complete() {
    let registry = crate::plugins::agent::capabilities::ops::get_registry();
    let op = registry.get("memory.reflect").unwrap();
    let schema = &op.meta().input_schema;
    let props = schema.get("properties").unwrap();
    assert!(props.get("summary").is_some(), "应有 summary 参数");
    assert!(props.get("insights").is_some(), "应有 insights 参数");
    assert!(
        props.get("rule_updates").is_some(),
        "应有 rule_updates 参数"
    );
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("summary")), "summary 应为必填");
}
