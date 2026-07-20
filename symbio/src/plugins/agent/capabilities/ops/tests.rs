//! ops/mod.rs 单元测试
//!
//! 对应源文件: `ops/mod.rs`

use super::*;

/// 验证所有操作都通过 inventory 自注册成功
#[test]
fn all_ops_registered() {
    let registry = get_registry();
    let ops = registry.registered_ops();

    let expected = vec![
        "memory.save",
        "memory.retrieve",
        "memory.graph_query",
        "memory.reflect",
        "memory.consolidate",
    ];

    assert_eq!(ops.len(), 5, "应注册 5 个操作，实际 {}", ops.len());
    for name in &expected {
        assert!(ops.contains(name), "缺少操作: {}", name);
    }
}

/// 验证每个操作都有非空的 name 和 description
#[test]
fn all_ops_have_metadata() {
    let registry = get_registry();
    for (name, op) in &registry.ops {
        let m = op.meta();
        assert!(!name.is_empty(), "操作名不能为空");
        assert!(
            !m.description.is_empty(),
            "操作 {} 的 description 不能为空",
            name
        );
        assert!(
            m.description.len() > 10,
            "操作 {} 的 description 过短: {}",
            name,
            m.description
        );
    }
}

/// 验证未知操作返回错误
#[tokio::test]
async fn execute_unknown_op_returns_error() {
    let registry = get_registry();
    assert!(
        registry.get("nonexistent.op").is_none(),
        "未知操作应返回 None"
    );
}

/// 验证 registry 路由到正确的操作
#[tokio::test]
async fn registry_routes_correctly() {
    let registry = get_registry();

    assert!(
        registry.get("memory.save").is_some(),
        "memory.save 应已注册"
    );
    assert!(
        registry.get("memory.retrieve").is_some(),
        "memory.retrieve 应已注册"
    );
    assert!(registry.get("unknown.op").is_none(), "未知操作应返回 None");
}

// ── 辅助：创建临时 SqliteStorage 作为 mock store ──
fn mock_store() -> Arc<dyn crate::plugins::agent::core::AgentStore> {
    crate::plugins::agent::store::build_in_memory_store()
}

// ── memory.save 测试（通过 store_unit 需要 Mindscape 层，此处测试元数据和注册）──

#[test]
fn memory_save_meta_schema() {
    let registry = get_registry();
    let op = registry.get("memory.save").unwrap();
    let schema = &op.meta().input_schema;
    let props = schema.get("properties").unwrap();
    assert!(props.get("items").is_some(), "应有 items 参数");
    assert!(
        props.get("content").is_none(),
        "不应有 content 参数（已统一为 items）"
    );
    assert!(
        props.get("target_id").is_none(),
        "不应有 target_id 参数（已统一为 items）"
    );
}

#[test]
fn memory_retrieve_meta_schema() {
    let registry = get_registry();
    let op = registry.get("memory.retrieve").unwrap();
    let schema = &op.meta().input_schema;
    let props = schema.get("properties").unwrap();
    assert!(props.get("filter").is_some(), "应有 filter 参数");
    assert!(props.get("limit").is_some(), "应有 limit 参数");
    assert!(
        props.get("query").is_none(),
        "不应有 query 参数（已统一为 filter.semantic）"
    );
    assert!(
        props.get("id").is_none(),
        "不应有 id 参数（已统一为 filter.id）"
    );
    assert!(
        props.get("type_filter").is_none(),
        "不应有 type_filter 参数（已统一为 filter.is_a）"
    );
}

#[test]
fn memory_retrieve_registered() {
    let registry = get_registry();
    assert!(
        registry.get("memory.retrieve").is_some(),
        "memory.retrieve 应已注册"
    );
}

#[test]
fn all_ops_have_nonempty_descriptions() {
    let registry = get_registry();
    for (name, op) in registry.iter() {
        let desc = &op.meta().description;
        assert!(!desc.is_empty(), "操作 {} 的 description 为空", name);
        assert!(
            desc.len() > 5,
            "操作 {} 的 description 过短: {}",
            name,
            desc
        );
    }
}

#[test]
fn all_ops_have_input_schema() {
    let registry = get_registry();
    for (name, op) in registry.iter() {
        let schema = &op.meta().input_schema;
        assert!(
            schema.is_object(),
            "操作 {} 的 input_schema 不是对象: {:?}",
            name,
            schema
        );
    }
}

// ── 基础 store 直接测试（不经过 store_unit）──

#[tokio::test]
async fn memory_storage_basic_crud() {
    let store = mock_store();
    use crate::plugins::agent::core::cu_from_json;
    use serde_json::json;

    let cu = cu_from_json(json!({"id": "test_001", "description": "测试", "confidence": 0.8}));
    store.insert(&cu).await.unwrap();

    let got = store.get("test_001").await.unwrap();
    assert!(got.is_some());
    assert_eq!(got.unwrap().description(), Some("测试"));

    store.delete("test_001").await.unwrap();
    assert!(store.get("test_001").await.unwrap().is_none());
}

#[tokio::test]
async fn mock_store_query_returns_total() {
    let store = mock_store();
    // 空 store 的 total 应为 0
    let page = store
        .query(
            &crate::plugins::agent::core::FilterExpr::match_all(),
            &crate::plugins::agent::core::PageRequest::first(10),
        )
        .await
        .unwrap();
    assert_eq!(page.total, 0, "空 store total 应为 0");
}

// ── 执行级测试：通过 MindscapeScaffold 完整栈测试 ops ──

#[tokio::test]
async fn op_memory_save_and_retrieve_via_scaffold() {
    use serde_json::json;
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let registry = get_registry();

    // save（使用 items 数组格式，不再支持顶层简写）
    let save_op = registry.get("memory.save").unwrap();
    let result = save_op
        .execute(
            engine.clone(),
            &json!({
                "items": [{
                    "is_a": ["fact"],
                    "description": "Rust 的所有权系统确保内存安全",
                    "confidence": 0.9
                }]
            }),
        )
        .await;
    assert!(result.success, "memory.save 应成功: {:?}", result.error);
    let id = result
        .data
        .as_ref()
        .unwrap()
        .get("id")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // retrieve by id（通过统一 filter）
    let retrieve_op = registry.get("memory.retrieve").unwrap();
    let retrieve_result = retrieve_op
        .execute(
            engine.clone(),
            &json!({
                "filter": {"id": id}
            }),
        )
        .await;
    assert!(
        retrieve_result.success,
        "memory.retrieve 应成功: {:?}",
        retrieve_result.error
    );
}

#[tokio::test]
async fn op_memory_save_batch_via_scaffold() {
    use serde_json::json;
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let registry = get_registry();

    let op = registry.get("memory.save").unwrap();
    let result = op
        .execute(
            engine.clone(),
            &json!({
                "items": [
                    {"is_a": ["fact"], "description": "事实1"},
                    {"is_a": ["rule"], "description": "规则1"}
                ]
            }),
        )
        .await;
    assert!(result.success, "save batch 应成功: {:?}", result.error);
    let count = result
        .data
        .as_ref()
        .unwrap()
        .get("count")
        .unwrap()
        .as_u64()
        .unwrap();
    assert_eq!(count, 2, "应保存 2 条记忆");
}

#[tokio::test]
async fn op_memory_soft_delete_via_scaffold() {
    use serde_json::json;
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let registry = get_registry();

    // 先保存
    let save_op = registry.get("memory.save").unwrap();
    let save_result = save_op
        .execute(
            engine.clone(),
            &json!({
                "items": [{
                    "id": "soft_del_001",
                    "is_a": ["fact"],
                    "description": "将被软删除的记忆"
                }]
            }),
        )
        .await;
    assert!(save_result.success, "save 应成功: {:?}", save_result.error);

    // 验证已存在
    let cu = engine.get("soft_del_001").await.unwrap().unwrap();
    assert_eq!(cu.id(), "soft_del_001");

    // 软删除：confidence=0 → 立即物理删除（不等 consolidate）
    let soft_del_result = save_op
        .execute(
            engine.clone(),
            &json!({
                "items": [{
                    "id": "soft_del_001",
                    "confidence": 0
                }]
            }),
        )
        .await;
    assert!(
        soft_del_result.success,
        "soft delete 应成功: {:?}",
        soft_del_result.error
    );
    let data = soft_del_result.data.unwrap();
    assert_eq!(
        data["action"], "deleted",
        "soft delete 应返回 action=deleted"
    );

    // 验证已被立即删除（不需 consolidate）
    let get_result = engine.get("soft_del_001").await.unwrap();
    assert!(get_result.is_none(), "soft delete 后应立即不存在");
}

#[tokio::test]
async fn op_memory_stats_via_scaffold() {
    use serde_json::json;
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let registry = get_registry();

    // 保存 3 条
    let save_op = registry.get("memory.save").unwrap();
    for i in 0..3 {
        let r = save_op
            .execute(
                engine.clone(),
                &json!({
                    "items": [{
                        "is_a": ["fact"],
                        "description": format!("记忆 {}", i)
                    }]
                }),
            )
            .await;
        assert!(r.success, "save 应成功: {:?}", r.error);
    }

    // retrieve 无 filter 浏览，验证 total
    let retrieve_op = registry.get("memory.retrieve").unwrap();
    let result = retrieve_op
        .execute(engine.clone(), &json!({"limit": 10}))
        .await;
    assert!(result.success, "retrieve 应成功: {:?}", result.error);
    let total = result
        .data
        .as_ref()
        .unwrap()
        .get("total")
        .unwrap()
        .as_u64()
        .unwrap();
    assert!(total >= 3, "应至少有 3 条记忆，实际 {}", total);
}

#[tokio::test]
async fn op_memory_retrieve_browse_via_scaffold() {
    use serde_json::json;
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let registry = get_registry();

    // 保存 2 条
    let save_op = registry.get("memory.save").unwrap();
    save_op
        .execute(
            engine.clone(),
            &json!({"items": [{"is_a": ["fact"], "description": "记忆A"}]}),
        )
        .await;
    save_op
        .execute(
            engine.clone(),
            &json!({"items": [{"is_a": ["rule"], "description": "记忆B"}]}),
        )
        .await;

    // 无 filter 浏览
    let retrieve_op = registry.get("memory.retrieve").unwrap();
    let result = retrieve_op
        .execute(engine.clone(), &json!({"limit": 10}))
        .await;
    assert!(result.success, "retrieve browse 应成功: {:?}", result.error);
}

#[tokio::test]
async fn op_memory_save_is_a_sets_correctly() {
    use serde_json::json;
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let registry = get_registry();

    // 保存一条 rule 类型的认知单元
    let save_op = registry.get("memory.save").unwrap();
    let result = save_op
        .execute(
            engine.clone(),
            &json!({
                "items": [{
                    "is_a": ["rule"],
                    "description": "代码规范"
                }]
            }),
        )
        .await;
    assert!(result.success, "save 应成功: {:?}", result.error);
    let id = result
        .data
        .as_ref()
        .unwrap()
        .get("id")
        .unwrap()
        .as_str()
        .unwrap();

    // 通过 store 直接验证 is_a
    let stored = engine.get(id).await.unwrap().unwrap();
    assert!(
        stored.is_type("rule"),
        "is_a=['rule'] 应映射正确，实际: {:?}",
        stored.is_a_list()
    );
}

#[tokio::test]
async fn op_memory_retrieve_total_returns_count() {
    use serde_json::json;
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let registry = get_registry();

    let op = registry.get("memory.retrieve").unwrap();
    let result = op.execute(engine.clone(), &json!({"limit": 10})).await;
    assert!(result.success, "memory.retrieve 应成功: {:?}", result.error);
    assert!(
        result.data.as_ref().unwrap().get("total").is_some(),
        "应返回 total 字段"
    );
}
