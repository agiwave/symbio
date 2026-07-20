//! cognition.rs 单元测试
//!
//! 对应源文件: `cognition.rs`

use super::*;

#[test]
fn cognition_request_memory_save() {
    // 使用 items 数组格式（顶层简写已移除）
    let req: CognitionRequest = serde_json::from_str(
        r#"{
        "operation": "memory.save",
        "items": [{
            "is_a": ["fact"],
            "description": "Rust 是系统编程语言",
            "confidence": 0.9
        }]
    }"#,
    )
    .unwrap();
    assert_eq!(req.operation, "memory.save");
    assert!(req.params.get("items").is_some(), "应包含 items 数组");
}

#[test]
fn cognition_request_memory_retrieve() {
    // I-051 修复：使用实际的 filter 字段
    let req: CognitionRequest = serde_json::from_str(
        r#"{
        "operation": "memory.retrieve",
        "filter": {"semantic": "Rust 所有权", "is_a": "fact"},
        "limit": 5
    }"#,
    )
    .unwrap();
    assert_eq!(req.operation, "memory.retrieve");
    assert!(req.params.get("filter").is_some(), "应包含 filter");
    assert_eq!(req.params.get("limit").and_then(|v| v.as_u64()), Some(5));
}

#[test]
fn cognition_request_memory_soft_delete() {
    // 软删除：confidence=0（已废除 memory.delete）
    let req: CognitionRequest = serde_json::from_str(
        r#"{
        "operation": "memory.save",
        "items": [{"id": "cu_001", "confidence": 0}]
    }"#,
    )
    .unwrap();
    assert_eq!(req.operation, "memory.save");
    let items = req.params.get("items").and_then(|v| v.as_array()).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get("confidence").and_then(|v| v.as_f64()),
        Some(0.0)
    );
}

#[test]
fn cognition_request_memory_graph_query() {
    let req: CognitionRequest = serde_json::from_str(
        r#"{
        "operation": "memory.graph_query",
        "graph_operation": "neighbors",
        "node_id": "cu_xxx",
        "max_hops": 2
    }"#,
    )
    .unwrap();
    assert_eq!(req.operation, "memory.graph_query");
    assert_eq!(
        req.params.get("graph_operation").and_then(|v| v.as_str()),
        Some("neighbors")
    );
    assert_eq!(
        req.params.get("node_id").and_then(|v| v.as_str()),
        Some("cu_xxx")
    );
}

#[test]
fn cognition_request_only_operation() {
    let req: CognitionRequest =
        serde_json::from_str(r#"{"operation": "memory.retrieve"}"#).unwrap();
    assert_eq!(req.operation, "memory.retrieve");
    assert!(req.params.is_empty(), "无参数时 params 应为空");
}

#[test]
fn operation_format_validation() {
    let req: CognitionRequest = serde_json::from_str(r#"{"operation": "memory.save"}"#).unwrap();
    assert_eq!(req.operation, "memory.save");

    let req: CognitionRequest = serde_json::from_str(r#"{"operation": "invalid"}"#).unwrap();
    assert_eq!(req.operation, "invalid");
}

// ── schema 测试 ──

#[test]
fn schema_structure() {
    let schema = AgentCognitionTool::build_schema();

    let props = schema["properties"].as_object().unwrap();
    assert_eq!(props.len(), 1, "schema 应只有 operation 一个属性");
    assert!(
        props.contains_key("operation"),
        "schema 应包含 operation 属性"
    );
    assert_eq!(
        schema["additionalProperties"], true,
        "schema 应允许额外参数"
    );

    let required = schema["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0], "operation");
}

#[test]
fn schema_operation_description_contains_all_ops() {
    let schema = AgentCognitionTool::build_schema();
    let desc = schema["properties"]["operation"]["description"]
        .as_str()
        .unwrap();

    let expected_ops = [
        "memory.save",
        "memory.retrieve",
        "memory.graph_query",
        "memory.reflect",
        "memory.consolidate",
    ];
    for op in &expected_ops {
        assert!(desc.contains(op), "schema 描述中缺少操作: {}", op);
    }

    // I-052 修复：现在 schema 描述指向"详细参数见 input_schema"
    assert!(
        desc.contains("JSON Schema") || desc.contains("提示"),
        "schema 描述应包含参数指引"
    );
}

#[test]
fn schema_description_contains_op_names() {
    let schema = AgentCognitionTool::build_schema();
    let desc = schema["properties"]["operation"]["description"]
        .as_str()
        .unwrap();

    assert!(
        desc.contains("memory.save"),
        "schema 描述应包含 memory.save"
    );
    assert!(
        desc.contains("memory.retrieve"),
        "schema 描述应包含 memory.retrieve"
    );
}

// ── 参数校验测试 ──

/// 构造一个 CapabilityMeta 辅助测试
fn make_meta(name: &str, desc: &str, required: Vec<&str>) -> CapabilityMeta {
    let mut properties = serde_json::Map::new();
    for r in &required {
        properties.insert(r.to_string(), serde_json::json!({"type": "string"}));
    }
    CapabilityMeta {
        name: name.to_string(),
        description: desc.to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
        }),
        ..Default::default()
    }
}

#[test]
fn check_required_params_missing() {
    let meta = make_meta(
        "memory.save",
        "保存记忆。示例：{description:'...', confidence:0.9}",
        vec!["content"],
    );
    let params = serde_json::json!({"type": "semantic"});
    let hint = AgentCognitionTool::check_required_params("memory.save", &meta, &params);
    assert!(hint.is_some(), "缺少必需参数应返回提示");
    let msg = hint.unwrap();
    assert!(msg.contains("content"), "提示应包含缺失的参数名");
    assert!(msg.contains("示例"), "提示应包含该 op 的完整 description");
}

#[test]
fn check_required_params_present() {
    let meta = make_meta("memory.save", "保存记忆。", vec!["content"]);
    let params = serde_json::json!({"content": "hello"});
    let hint = AgentCognitionTool::check_required_params("memory.save", &meta, &params);
    assert!(hint.is_none(), "参数齐全不应返回提示");
}

#[test]
fn check_required_no_schema_required() {
    let meta = make_meta("memory.retrieve", "检索记忆。", vec![]);
    let params = serde_json::json!({});
    let hint = AgentCognitionTool::check_required_params("memory.retrieve", &meta, &params);
    assert!(hint.is_none(), "无必需参数的 op 不应返回提示");
}

// ── 分发集成测试 ──

/// 模拟 LLM 调用：验证 CognitionRequest 能正确解析 LLM 发送的 JSON
#[test]
fn llm_call_simulation_flat_json() {
    let calls = vec![
        // ✅ 正确：items 数组格式
        (
            r#"{"operation": "memory.save", "items": [{"description": "Rust is fast"}]}"#,
            "memory.save",
        ),
        (
            r#"{"operation": "memory.save", "items": [{"id": "cu_xxx", "name": "新名字"}]}"#,
            "memory.save",
        ),
        (
            r#"{"operation": "memory.save", "items": [{"description": "a"}, {"description": "b"}]}"#,
            "memory.save",
        ),
        (
            r#"{"operation": "memory.delete", "ids": ["cu_001", "cu_002"]}"#,
            "memory.delete",
        ),
        (
            r#"{"operation": "memory.retrieve", "filter": {"is_a": "fact"}, "limit": 10}"#,
            "memory.retrieve",
        ),
    ];

    for (json_str, expected_op) in calls {
        let req: CognitionRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req.operation, expected_op, "operation 不匹配: {}", json_str);
    }
}

/// 验证统一参数校验覆盖所有有必需参数的 op
#[test]
fn unified_param_check_covers_all_ops() {
    let registry = crate::plugins::agent::capabilities::ops::get_registry();
    for (name, op) in registry.iter() {
        let m = op.meta();
        let required: Vec<&str> = m
            .input_schema
            .get("required")
            .and_then(|v: &Value| v.as_array())
            .map(|arr: &Vec<Value>| arr.iter().filter_map(|v: &Value| v.as_str()).collect())
            .unwrap_or_default();

        if !required.is_empty() {
            // 空 params 应触发校验
            let params = serde_json::json!({});
            let hint = AgentCognitionTool::check_required_params(name, &m, &params);
            assert!(
                hint.is_some(),
                "op '{}' 有必需参数 {:?} 但校验未触发",
                name,
                required
            );
            let hint_text = hint.unwrap();
            for field in &required {
                assert!(
                    hint_text.contains(field),
                    "op '{}' 提示缺少字段 {}: {}",
                    name,
                    field,
                    hint_text
                );
            }
        }
    }
}
