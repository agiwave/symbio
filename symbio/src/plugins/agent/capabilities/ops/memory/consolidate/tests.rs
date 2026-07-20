//! memory/consolidate.rs 单元测试
//!
//! 对应源文件: `memory/consolidate.rs`

use super::*;
use crate::plugins::agent::capabilities::ops::CognitionOp;

/// dry_run 模式：应返回报告但不执行写操作
#[tokio::test]
async fn consolidate_dry_run_returns_report() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;
    let op = ConsolidateOp;

    let result = op.execute(engine.clone(), &json!({"dry_run": true})).await;

    assert!(result.success, "dry_run 应成功: {:?}", result.error);
    let data = result.data.unwrap();
    assert_eq!(data["status"], "dry_run");
    assert_eq!(data["dry_run"], true);
    assert!(data.get("would_forget").is_some(), "应有遗忘报告");
    assert!(data.get("would_merge").is_some(), "应有合并报告");
    assert!(data.get("would_promote").is_some(), "应有晋升报告");
    assert!(
        data.get("candidate_pool_health").is_some(),
        "应有候选池健康报告"
    );
}

/// 遗忘衰减：创建一个 belief 很低、很久没访问、priority>20 的 CU，应被遗忘
#[tokio::test]
async fn consolidate_forgets_low_retention_cu() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;

    // 创建一个 priority>20、低 belief、很久以前的 CU
    let mut cu = CognitiveUnit::generate_id();
    cu.set_name("过时知识");
    cu.set_description("将被遗忘");
    cu.add_type("fact");
    // priority=200 > 20 → 不进入提示词 → 可被遗忘
    cu.set(cu_fields::PRIORITY, json!(200));
    cu.set_confidence(0.1);
    cu.set_meta_belief(0.05);
    // 设置 last_access 为很久以前（30天前）
    let old_time = now_secs() - (30 * 86400);
    cu.set("_ext_last_access", json!(old_time));
    cu.set("_ext_memory_strength", json!(1.0)); // 1 天的 memory_strength → 急速衰减
    let id = cu.id().to_string();
    engine.upsert(&cu).await.unwrap();

    // 执行 consolidate
    let op = ConsolidateOp;
    let result = op.execute(engine.clone(), &json!({"dry_run": false})).await;

    assert!(result.success);
    let data = result.data.unwrap();
    let forgotten_ids: Vec<String> = data["forgotten_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert!(forgotten_ids.contains(&id), "低保留度 CU 应被遗忘");

    // 验证 CU 已被删除
    let got = engine.get(&id).await.unwrap();
    assert!(got.is_none(), "被遗忘的 CU 应已从 store 删除");
}

/// priority<=20 的 CU 不应被遗忘（即使 retention 很低）
#[tokio::test]
async fn consolidate_preserves_high_priority_cu() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;

    // 创建一个 priority<=20（候选池内）、低 belief 的 CU
    let mut cu = CognitiveUnit::generate_id();
    cu.set_name("重要规则");
    cu.set_description("不应被遗忘");
    cu.add_type("rule");
    // priority=10（默认）→ 在候选池中 → 应受保护
    cu.set(cu_fields::PRIORITY, json!(10));
    cu.set_meta_belief(0.01);
    let old_time = now_secs() - (365 * 86400);
    cu.set("_ext_last_access", json!(old_time));
    cu.set("_ext_memory_strength", json!(1.0));
    let id = cu.id().to_string();
    engine.upsert(&cu).await.unwrap();

    let op = ConsolidateOp;
    let result = op.execute(engine.clone(), &json!({"dry_run": false})).await;

    assert!(result.success);
    let data = result.data.unwrap();
    let forgotten_ids: Vec<String> = data["forgotten_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert!(!forgotten_ids.contains(&id), "高 priority CU 不应被遗忘");

    // 验证 CU 仍在
    let got = engine.get(&id).await.unwrap();
    assert!(got.is_some(), "高 priority CU 应仍存在");
}

/// 优先级晋升：高 access_count + 高 priority 的 CU 应被晋升
#[tokio::test]
async fn consolidate_promotes_high_access_cu() {
    let engine = crate::plugins::agent::store::build_test_scaffold().await;

    let mut cu = CognitiveUnit::generate_id();
    cu.set_name("高频知识");
    cu.set_description("经常被检索");
    cu.add_type("fact");
    // priority=20（原值），access_count 触发晋升到 priority < 20
    cu.set("priority", json!(20));
    cu.set("_ext_access_count", json!(15));
    let id = cu.id().to_string();
    engine.upsert(&cu).await.unwrap();

    let op = ConsolidateOp;
    let result = op.execute(engine.clone(), &json!({"dry_run": false})).await;

    assert!(result.success);
    let data = result.data.unwrap();
    assert_eq!(data["promoted_count"], 1);

    // 验证 priority 已降低
    let updated = engine.get(&id).await.unwrap().unwrap();
    let new_priority = updated
        .get("priority")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert_eq!(new_priority, 15, "priority 应从 20 降到 15");
}

/// 遗忘曲线公式验证
#[test]
fn retention_formula() {
    // belief=0.5, delta=30天, memory_strength=30天 → retention = 0.5 * exp(-1) ≈ 0.184
    let belief: f64 = 0.5;
    let delta_days: f64 = 30.0;
    let memory_strength: f64 = 30.0;
    let retention = belief * (-delta_days / memory_strength).exp();
    assert!((retention - 0.1839).abs() < 0.01);

    // belief=0.5, delta=90天, memory_strength=30天 → retention = 0.5 * exp(-3) ≈ 0.025
    let retention2 = belief * (-90.0_f64 / memory_strength).exp();
    assert!((retention2 - 0.0249).abs() < 0.01);
}

/// op 已注册到 registry
#[test]
fn consolidate_registered_in_registry() {
    let registry = crate::plugins::agent::capabilities::ops::get_registry();
    assert!(
        registry.get("memory.consolidate").is_some(),
        "memory.consolidate 应已注册"
    );
}
