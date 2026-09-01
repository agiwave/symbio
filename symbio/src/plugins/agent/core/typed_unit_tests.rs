use super::*;
use serde_json::json;

#[test]
fn test_new_unit_basics() {
    let u = CognitiveUnit::new("test_id");
    assert_eq!(u.id(), "test_id");
    assert_eq!(u.confidence(), 0.5);
    assert_eq!(u.meta_belief(), 0.5);
    assert!(u.name().is_none());
}

#[test]
fn test_structured_field_access() {
    let mut u = CognitiveUnit::new("identity");
    u.set_name("张三");
    u.set_description("我是张三");
    u.set_content("自我介绍");
    u.set_confidence(0.9);
    u.set_meta_belief(0.8);

    assert_eq!(u.id(), "identity");
    assert_eq!(u.name(), Some("张三"));
    assert_eq!(u.description(), Some("我是张三"));
    assert_eq!(u.content(), Some("自我介绍"));
    assert_eq!(u.confidence(), 0.9);
    assert_eq!(u.meta_belief(), 0.8);

    // 清除
    u.clear_name();
    assert!(u.name().is_none());
}

#[test]
fn test_confidence_and_meta_belief_clamped() {
    let mut u = CognitiveUnit::new("x");
    u.set_confidence(2.0);
    assert_eq!(u.confidence(), 1.0);
    u.set_confidence(-1.0);
    assert_eq!(u.confidence(), 0.0);

    u.set_meta_belief(1.0);
    assert_eq!(u.meta_belief(), 0.99);
    u.bump_meta_belief(0.5);
    assert_eq!(u.meta_belief(), 0.99); // 仍然 cap 在 0.99
}

#[test]
fn test_relations_mechanism() {
    let mut u = CognitiveUnit::new("rain");
    u.add_relation("is_a", "fact");
    u.add_relation("is_a", "rule");
    assert!(u.is_type("fact"));
    assert!(u.is_type("rule"));

    // 任意关系名
    u.add_relation("causes", "wet_ground");
    u.add_relation("custom_relationship", "target_x");
    assert_eq!(u.relations("causes"), vec!["wet_ground".to_string()]);
    assert!(u.has_relation("custom_relationship", "target_x"));

    // 去重
    u.add_relation("causes", "wet_ground");
    assert_eq!(u.relations("causes").len(), 1);

    // 移除（清空后自动删除键）
    u.remove_relation("is_a", "rule");
    assert!(!u.is_type("rule"));
    assert!(u.is_type("fact"));
    u.remove_relation("is_a", "fact");
    assert!(u.data.get("is_a").is_none());
}

#[test]
fn test_is_type_convenience() {
    let mut u = CognitiveUnit::new("x");
    u.set_types(vec!["rule", "fact"]);
    assert!(u.is_type("rule"));
    assert!(u.is_type("fact"));
    assert!(!u.is_type("skill"));
    assert!(u.is_any_type(&["skill", "fact"]));
}

#[test]
fn test_to_llm_value_excludes_meta() {
    let mut u = CognitiveUnit::new("identity");
    u.set_name("张三");
    u.set_description("desc");
    u.bump_version();
    u.record_access();

    let llm = u.to_llm_value();
    assert_eq!(llm.get("id").and_then(|v| v.as_str()), Some("identity"));
    assert_eq!(llm.get("name").and_then(|v| v.as_str()), Some("张三"));
    // _ext_* 不应出现
    assert!(llm.get("_ext_version").is_none());
    assert!(llm.get("_ext_created_at").is_none());
    assert!(llm.get("_ext_updated_at").is_none());
    assert!(llm.get("_ext_last_access").is_none());
}

#[test]
fn test_serialization_roundtrip() {
    let mut u = CognitiveUnit::new("identity");
    u.set_name("张三");
    u.set_description("desc");
    u.set_confidence(0.7);
    u.set_meta_belief(0.6);
    u.add_relation("is_a", "fact");
    u.add_relation("causes", "flood");
    u.set_prop_value_is_a("cu");
    u.bump_version();

    // → Value → 序列化
    let v = u.to_value();
    let yaml = serde_yaml_ng::to_string(&v).unwrap();

    // 反序列化：Value → CU
    let parsed_v: Value = serde_yaml_ng::from_str(&yaml).unwrap_or(v.clone());
    let parsed = CognitiveUnit::try_from(parsed_v).unwrap();
    assert_eq!(parsed.id(), "identity");
    assert_eq!(parsed.name(), Some("张三"));
    assert_eq!(parsed.confidence(), 0.7);
    assert_eq!(parsed.meta_belief(), 0.6);
    assert!(parsed.is_type("fact"));
    assert_eq!(parsed.relations("causes"), vec!["flood".to_string()]);
    assert_eq!(parsed.prop_value_is_a(), Some("cu"));
    assert!(parsed.version() >= 2);
}

#[test]
fn test_apply_update_merges_typed_fields() {
    let mut u = CognitiveUnit::new("identity");
    u.set_name("原名");
    u.set_description("原描述");
    u.apply_update(&json!({
        "name": "张三",
        "description": "新描述",
        "confidence": 0.95
    }))
    .unwrap();
    assert_eq!(u.name(), Some("张三"));
    assert_eq!(u.description(), Some("新描述"));
    assert!((u.confidence() - 0.95).abs() < 0.001);
}

#[test]
fn test_apply_update_protects_id_and_meta() {
    let mut u = CognitiveUnit::new("identity");
    u.set_name("原名");
    u.apply_update(&json!({
        "id": "evil",
        "name": "张三",
        "_ext_version": 999,
        "_ext_last_access": 12345
    }))
    .unwrap();
    assert_eq!(u.id(), "identity");
    assert_eq!(u.name(), Some("张三"));
    // _ext_version 不应被改
    assert_ne!(u.version(), 999);
}

#[test]
fn test_apply_update_merges_relations() {
    let mut u = CognitiveUnit::new("rain");
    u.apply_update(&json!({
        "name": "雨",
        "is_a": ["fact", "weather"],
        "causes": ["wet", "flood"],
        "depends": ["cloud"]
    }))
    .unwrap();
    assert_eq!(u.name(), Some("雨"));
    assert!(u.is_type("fact"));
    assert!(u.is_type("weather"));
    assert_eq!(
        u.relations("causes"),
        vec!["wet".to_string(), "flood".to_string()]
    );
    assert_eq!(u.relations("depends"), vec!["cloud".to_string()]);
}

#[test]
fn test_apply_update_merges_arbitrary_keys() {
    // **v9.4 关键测试**：apply_update 直接合并任意键到 data。
    // 没有"properties 隔离"概念——所有键都是 data 的一等公民。
    let mut u = CognitiveUnit::new("cu_1");
    u.apply_update(&json!({
        "name": "any",
        "custom_tag": "value_a",
        "source": "user_input",
        "nested": { "a": 1 }
    }))
    .unwrap();
    assert_eq!(u.name(), Some("any"));
    assert_eq!(
        u.get("custom_tag").and_then(|v| v.as_str()),
        Some("value_a")
    );
    assert_eq!(u.get("source").and_then(|v| v.as_str()), Some("user_input"));
    // 嵌套对象直接存
    assert!(u.get("nested").map(|v| v.is_object()).unwrap_or(false));
}

#[test]
fn test_cognitive_unit_priority_field() {
    // priority 字段决定"是否进入系统提示词"和"在提示词中的顺序"
    let mut u = CognitiveUnit::new("x");
    assert_eq!(
        u.get_number(cu_fields::PRIORITY),
        None,
        "默认无 priority 字段"
    );

    u.set(cu_fields::PRIORITY, json!(0));
    assert_eq!(
        u.get_number(cu_fields::PRIORITY),
        Some(0.0),
        "priority=0：强制入提示词最前"
    );

    u.set(cu_fields::PRIORITY, json!(200));
    assert_eq!(
        u.get_number(cu_fields::PRIORITY),
        Some(200.0),
        "priority=200：不入提示词"
    );
}

#[test]
fn test_from_value_requires_id() {
    assert!(CognitiveUnit::from_value(json!({"name": "x"})).is_err());
    assert!(CognitiveUnit::from_value(json!("plain string")).is_err());
    let ok = CognitiveUnit::from_value(json!({"id": "x"})).unwrap();
    assert_eq!(ok.id(), "x");
}

#[test]
fn test_get_set_remove_generic() {
    let mut u = CognitiveUnit::new("x");
    u.set("any_key", json!("any_value"));
    assert_eq!(u.get("any_key").and_then(|v| v.as_str()), Some("any_value"));
    assert!(u.contains("any_key"));
    let removed = u.remove("any_key");
    assert_eq!(removed, Some(json!("any_value")));
    assert!(!u.contains("any_key"));
}

#[test]
fn test_embedding_sanitized() {
    let mut u = CognitiveUnit::new("x");
    u.set_embedding(vec![0.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.3]);
    let stored = u.embedding().unwrap();
    assert_eq!(stored.len(), 5);
    for v in &stored {
        assert!(v.is_finite());
    }
    // v16: Q8 量化后反量化，误差 ≤ 1 LSB ≈ scale
    // range = [0, 0.5]（NaN/Inf→0，-0.3 是最小值），scale ≈ 0.5/255
    // 0.5 → q=255 → recovered ≈ 0.4980（在 0.5±0.005 范围内）
    let scale = (0.5 - (-0.3)) / 255.0;
    assert!(
        (stored[0] - 0.5).abs() <= scale + 1e-5,
        "stored[0]={}",
        stored[0]
    );
    // NaN/Inf 经 sanitization 替 0，再经 Q8 量化 → recovered ≈ 0
    assert!(stored[1].abs() <= scale + 1e-5, "stored[1]={}", stored[1]);
    assert!(stored[2].abs() <= scale + 1e-5, "stored[2]={}", stored[2]);
    assert!(stored[3].abs() <= scale + 1e-5, "stored[3]={}", stored[3]);
    assert!(
        (stored[4] - (-0.3)).abs() <= scale + 1e-5,
        "stored[4]={}",
        stored[4]
    );
    // Q8 标志位
    assert!(u.is_embedding_q8());
}

#[test]
fn test_embedding_legacy_array_format_compat() {
    // 模拟旧格式数据：直接插入 f32 数组到 _ext_embedding
    let mut u = CognitiveUnit::new("legacy");
    let legacy = Value::Array(
        vec![1.0_f32, 2.0, 3.0]
            .into_iter()
            .map(|f| {
                serde_json::Number::from_f64(f as f64)
                    .map(Value::Number)
                    .unwrap()
            })
            .collect(),
    );
    u.data.insert("_ext_embedding".to_string(), legacy);
    assert!(!u.is_embedding_q8());
    // v19 修复：legacy 格式直接读为 f32 数组，**不走 Q8 转换**
    let rec = u.embedding().unwrap();
    assert_eq!(rec, vec![1.0_f32, 2.0, 3.0]);
}

#[test]
fn test_embedding_legacy_negative_values_preserved() {
    // v19 关键回归测试：旧格式带负值必须**完整保留**（不丢精度）
    let mut u = CognitiveUnit::new("legacy_neg");
    let legacy = Value::Array(
        vec![-0.3_f32, 0.5, 1.2, -1.5, 0.001]
            .into_iter()
            .map(|f| {
                serde_json::Number::from_f64(f as f64)
                    .map(Value::Number)
                    .unwrap()
            })
            .collect(),
    );
    u.data.insert("_ext_embedding".to_string(), legacy);
    assert!(!u.is_embedding_q8());
    let rec = u.embedding().expect("legacy must decode");
    // 关键：负值必须保留
    assert_eq!(rec[0], -0.3, "v18 静默丢失负值（clamp 到 0）— v19 已修复");
    assert_eq!(rec[1], 0.5, "v18 把 0.5 round 到 1 — v19 已修复");
    assert_eq!(rec[2], 1.2, "v18 把 1.2 round 到 1 — v19 已修复");
    assert_eq!(rec[3], -1.5);
    assert!((rec[4] - 0.001).abs() < 1e-5);
}

#[test]
fn test_set_embedding_q8_raw() {
    let mut u = CognitiveUnit::new("raw");
    u.set_embedding_q8_raw(vec![0, 64, 128, 192, 255], 0.01, 0.0);
    assert!(u.is_embedding_q8());
    let rec = u.embedding().unwrap();
    // v19 验证 _format_version 字段存在
    let stored = u.data.get("_ext_embedding").unwrap();
    let obj = stored.as_object().unwrap();
    assert_eq!(
        obj.get("_format_version").and_then(|x| x.as_u64()),
        Some(2),
        "v19 写入应包含 _format_version=2"
    );
    // 反量化：(0-0)*0.01=0, (64-0)*0.01=0.64, ..., (255-0)*0.01=2.55
    assert!((rec[0] - 0.0).abs() < 1e-6);
    assert!((rec[1] - 0.64).abs() < 1e-6);
    assert!((rec[2] - 1.28).abs() < 1e-6);
    assert!((rec[3] - 1.92).abs() < 1e-6);
    assert!((rec[4] - 2.55).abs() < 1e-6);
}

#[test]
fn test_set_embedding_q8_raw_nan_scale_sanitized() {
    // v19：NaN scale 经 q8_to_value 内部 sanitize 后变 0，反量化不会产生 NaN
    let mut u = CognitiveUnit::new("nan_test");
    u.set_embedding_q8_raw(vec![0, 128, 255], f32::NAN, 0.0);
    // 应能被读出（不会 pan ic）
    let rec = u.embedding().expect("NaN scale should be sanitized to 0");
    assert_eq!(rec.len(), 3);
    // 存储中的 scale 应该是 0（被 sanitize 替换）
    let stored = u.data.get("_ext_embedding").unwrap();
    let obj = stored.as_object().unwrap();
    assert_eq!(obj.get("scale").and_then(|x| x.as_f64()), Some(0.0));
}

#[test]
fn test_clear_embedding() {
    let mut u = CognitiveUnit::new("c");
    u.set_embedding(vec![1.0, 2.0, 3.0]);
    assert!(u.embedding().is_some());
    u.clear_embedding();
    assert!(u.embedding().is_none());
    assert!(!u.is_embedding_q8());
}

#[test]
fn test_version_and_access() {
    let mut u = CognitiveUnit::new("x");
    assert_eq!(u.version(), 1);
    u.bump_version();
    assert_eq!(u.version(), 2);
    u.record_access();
    assert_eq!(u.access_count(), 1);
    assert!(u.last_access().is_some());
}

#[test]
fn test_text_for_embedding() {
    let mut u = CognitiveUnit::new("test");
    u.set_name("Test");
    u.set_description("Description");
    u.set_content("Content");
    assert_eq!(
        u.text_for_embedding(),
        Some("Test Description Content".to_string())
    );

    let empty = CognitiveUnit::new("test");
    assert!(empty.text_for_embedding().is_none());
}

#[test]
fn test_no_properties_field_in_serialization() {
    // **v9.4 关键测试**：`properties` 字段**根本不存在**。
    // CU 的存储格式是 data 的直接序列化，YAML 中不应出现 `properties: {…}` 块。
    let mut u = CognitiveUnit::new("identity");
    u.set_name("张三");
    u.set("custom_field", json!("custom_value"));

    let v = u.to_value();
    let yaml = serde_yaml_ng::to_string(&v).unwrap();
    assert!(
        !yaml.contains("properties:"),
        "YAML 中绝不能出现 properties 块: {}",
        yaml
    );
    // custom_field 作为顶层字段出现
    assert!(
        yaml.contains("custom_field:") || yaml.contains("custom_field: "),
        "custom_field 应作为顶层字段: {}",
        yaml
    );
}

#[test]
fn test_equality_by_id() {
    let mut a = CognitiveUnit::new("x");
    a.set_name("name_a");
    let mut b = CognitiveUnit::new("x");
    b.set_name("name_b");
    // id 相同 ⇒ 相等
    assert_eq!(a, b);

    let c = CognitiveUnit::new("y");
    assert_ne!(a, c);
}

#[test]
fn test_is_relation_prop() {
    // 构造一个关系 prop CU
    let mut is_a_prop = CognitiveUnit::new("is_a");
    is_a_prop.add_relation("is_a", "relation");
    is_a_prop.set_prop_value_is_a("cu[]");
    assert!(is_a_prop.is_relation_prop());

    // 构造一个非关系 prop（prop_value_is_a 是 number）
    let mut confidence_prop = CognitiveUnit::new("confidence");
    confidence_prop.add_relation("is_a", "prop");
    confidence_prop.set_prop_value_is_a("number");
    assert!(!confidence_prop.is_relation_prop());

    // 构造一个 prop_value_is_a 是 cu[] 但 is_a 不含 relation 的
    let mut bad = CognitiveUnit::new("aliases");
    bad.add_relation("is_a", "prop");
    bad.set_prop_value_is_a("cu[]");
    assert!(!bad.is_relation_prop());

    // 构造一个 relation 类型但 prop_value_is_a 是 string 的
    let mut bad2 = CognitiveUnit::new("some_rel");
    bad2.add_relation("is_a", "relation");
    bad2.set_prop_value_is_a("string");
    assert!(!bad2.is_relation_prop());
}
