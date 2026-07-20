use super::*;
use crate::plugins::agent::core::{AgentStore, CognitiveUnit, FilterExpr, PageRequest};
use crate::plugins::agent::store::build_in_memory_store;
use crate::plugins::agent::store::mindscape::cognitive_feedback::CognitiveFeedback;
use crate::plugins::agent::store::mindscape::scaffold::VersionedSnapshot;
use std::collections::HashMap;
use std::sync::Arc as StdArc;
use std::time::Instant;

fn new_cu() -> CognitiveUnit {
    CognitiveUnit::new(crate::plugins::agent::core::types::generate_short_id())
}

/// 创建测试用单元（不依赖 LLM，纯确定性）
async fn make_test_unit(
    store: &StdArc<dyn AgentStore>,
    id: &str,
) -> crate::plugins::agent::core::CognitiveUnit {
    let mut u = new_cu();
    u.set_id(id);
    u.set_name(id);
    u.set_description(format!("test unit {}", id));
    u.add_relation("is_a", "test");
    u.set("level", serde_json::Value::String("msg".to_string()));
    u.set("meta_belief", serde_json::json!(0.5));
    store.insert(&u).await.unwrap()
}

/// Drop 兜底 - 空 buffer 时 Drop 不 panic
#[tokio::test]
async fn test_drop_scaffold_with_empty_buffer_does_not_panic() {
    let store: StdArc<dyn AgentStore> = build_in_memory_store();
    make_test_unit(&store, "u1").await;

    let feedback =
        crate::plugins::agent::store::mindscape::cognitive_feedback::CognitiveFeedback::new(
            store.clone(),
        );

    let scaffold = MindscapeScaffold {
        store,

        feedback,
        snapshot_cache: tokio::sync::RwLock::new(StdArc::new(VersionedSnapshot {
            data: HashMap::new(),
            created_at: Instant::now(),
            ..Default::default()
        })),
    };
    drop(scaffold);
}

/// Drop 兜底 - 有积压时 Drop 尝试 flush
#[tokio::test]
async fn test_drop_scaffold_with_pending_buffer_flushes() {
    let store: StdArc<dyn AgentStore> = build_in_memory_store();
    make_test_unit(&store, "u1").await;

    let feedback =
        crate::plugins::agent::store::mindscape::cognitive_feedback::CognitiveFeedback::new(
            store.clone(),
        );
    for _ in 0..100 {
        feedback.on_units_retrieved(&["u1"]).await;
    }
    let pending_before = feedback.pending_belief_updates().await;
    assert_eq!(pending_before, 1, "应有 1 项 buffer (id='u1', count=100)");

    // 正常路径：显式 flush → 返回 flushed count
    let flushed = feedback.flush_belief_buffer().await;
    assert_eq!(flushed, 1, "flush 应 flush 1 个 unit");
    let unit = store.get("u1").await.unwrap().unwrap();
    let belief = unit.get_number("meta_belief").unwrap();
    assert!(
        (belief - 0.99).abs() < 1e-6,
        "flush 应将 meta_belief 提升到 0.99（clamp），实际 {}",
        belief
    );

    let scaffold = MindscapeScaffold {
        store: store.clone(),

        feedback,
        snapshot_cache: tokio::sync::RwLock::new(StdArc::new(VersionedSnapshot {
            data: HashMap::new(),
            created_at: Instant::now(),
            ..Default::default()
        })),
    };
    drop(scaffold);
}

/// pending_belief_updates API 暴露
#[tokio::test]
async fn test_pending_belief_updates_returns_zero_when_empty() {
    let store: StdArc<dyn AgentStore> = build_in_memory_store();
    let feedback =
        crate::plugins::agent::store::mindscape::cognitive_feedback::CognitiveFeedback::new(store);
    let n = feedback.pending_belief_updates().await;
    assert_eq!(n, 0, "buffer 为空时 pending_belief_updates 应返回 0");
}

/// I-016：构造时 store 字段持有引用
#[tokio::test]
async fn test_scaffold_holds_store() {
    let store: StdArc<dyn AgentStore> = build_in_memory_store();
    let feedback =
        crate::plugins::agent::store::mindscape::cognitive_feedback::CognitiveFeedback::new(
            store.clone(),
        );

    let scaffold = MindscapeScaffold {
        store: store.clone(),
        feedback,
        snapshot_cache: tokio::sync::RwLock::new(StdArc::new(VersionedSnapshot {
            data: HashMap::new(),
            created_at: Instant::now(),
            ..Default::default()
        })),
    };

    assert!(StdArc::ptr_eq(&scaffold.store, &store));
    drop(scaffold);
}

/// I-016：snapshot_cache 字段可读写
#[tokio::test]
async fn test_snapshot_cache_read_write() {
    let store: StdArc<dyn AgentStore> = build_in_memory_store();
    let feedback =
        crate::plugins::agent::store::mindscape::cognitive_feedback::CognitiveFeedback::new(
            store.clone(),
        );

    let scaffold = MindscapeScaffold {
        store,

        feedback,
        snapshot_cache: tokio::sync::RwLock::new(StdArc::new(VersionedSnapshot {
            data: HashMap::new(),
            created_at: Instant::now(),
            ..Default::default()
        })),
    };

    let mut new_snap = VersionedSnapshot {
        data: HashMap::new(),
        created_at: Instant::now(),
        ..Default::default()
    };
    new_snap
        .data
        .insert("k1".to_string(), make_test_unit_simple());
    *scaffold.snapshot_cache.write().await = StdArc::new(new_snap);

    let snap = scaffold.snapshot_cache.read().await;
    assert_eq!(snap.data.len(), 1, "snapshot 写入后应能读到 1 个 unit");
    assert!(snap.data.contains_key("k1"));
}

fn make_test_unit_simple() -> CognitiveUnit {
    new_cu()
}

/// 辅助：构造完整的 MindscapeScaffold（用 in-memory store）
async fn make_scaffold() -> MindscapeScaffold {
    let store: StdArc<dyn AgentStore> = build_in_memory_store();
    let feedback = CognitiveFeedback::new(store.clone());

    MindscapeScaffold {
        store,

        feedback,
        snapshot_cache: tokio::sync::RwLock::new(StdArc::new(VersionedSnapshot {
            data: HashMap::new(),
            created_at: Instant::now(),
            ..Default::default()
        })),
    }
}

/// 辅助：构造带预置数据的 scaffold
async fn make_scaffold_with_data(ids: &[&str]) -> MindscapeScaffold {
    let scaffold = make_scaffold().await;
    for &id in ids {
        let mut u = new_cu();
        u.set_id(id);
        u.set_name(id);
        u.set_description(format!("test unit {}", id));
        u.add_relation("is_a", "test");
        u.set("level", serde_json::Value::String("msg".to_string()));
        u.set("meta_belief", serde_json::json!(0.5));
        scaffold.store.insert(&u).await.unwrap();
    }
    scaffold.invalidate_snapshot_cache().await;
    scaffold
}

// ─── upsert (CognitiveUnit) 测试 ───

/// upsert：创建新单元
#[tokio::test]
async fn test_upsert_creates_new() {
    let s = make_scaffold().await;
    let mut u = new_cu();
    u.set_id("new1");
    u.set_name("new1");
    u.set_description("test");
    u.add_relation("is_a", "fact");
    let result = s.upsert(&u).await;
    assert!(
        result.is_ok(),
        "upsert 创建新单元应成功: {:?}",
        result.err()
    );
    let stored = s.store.get("new1").await.unwrap();
    assert!(stored.is_some(), "创建后应能从 store 读到");
}

/// upsert：更新已有单元
#[tokio::test]
async fn test_upsert_updates_existing() {
    let s = make_scaffold_with_data(&["u1"]).await;
    let mut u = new_cu();
    u.set_id("u1");
    u.set_name("updated");
    u.set_description("updated desc");
    u.add_relation("is_a", "fact");
    let result = s.upsert(&u).await;
    assert!(result.is_ok(), "更新已有单元应成功: {:?}", result.err());
    let stored = s.store.get("u1").await.unwrap().unwrap();
    assert_eq!(stored.name(), Some("updated"), "更新后 name 应为 'updated'");
}

/// upsert：可正常修改 core 级单元（CognitiveUnit 接口不做保护）
#[tokio::test]
async fn test_upsert_modifies_core_level() {
    let s = make_scaffold().await;
    let mut u = new_cu();
    u.set_id("core1");
    u.set_name("core1");
    u.set("level", serde_json::Value::String("core".to_string()));
    s.store.insert(&u).await.unwrap();

    let mut updated = new_cu();
    updated.set_id("core1");
    updated.set_name("hack");
    let result = s.upsert(&updated).await;
    assert!(result.is_ok(), "CognitiveUnit 接口不阻止修改 core 级单元");
}

// ─── delete 测试 ───

/// delete：删除存在的单元
#[tokio::test]
async fn test_delete_existing() {
    let s = make_scaffold_with_data(&["d1"]).await;
    let result = s.delete("d1").await;
    assert!(result.is_ok(), "删除存在的单元应成功: {:?}", result.err());
    assert!(s.store.get("d1").await.unwrap().is_none(), "删除后应查不到");
}

/// delete：删除不存在的单元返回 Ok(false)
#[tokio::test]
async fn test_delete_not_found() {
    let s = make_scaffold().await;
    let result = s.delete("nonexistent").await;
    assert!(result.is_ok(), "删除不存在的单元应返回 Ok");
    assert!(!result.unwrap(), "不存在的单元 delete 应返回 false");
}

/// delete：CognitiveUnit 接口不阻止删除 core 级单元
#[tokio::test]
async fn test_delete_core_level() {
    let s = make_scaffold().await;
    let mut u = new_cu();
    u.set_id("core_del");
    u.set_name("core_del");
    u.set("level", serde_json::Value::String("core".to_string()));
    s.store.insert(&u).await.unwrap();

    let result = s.delete("core_del").await;
    assert!(result.is_ok(), "CognitiveUnit 接口不阻止删除 core 级单元");
}

// ─── query 测试 ───

/// query：按 is_a 过滤
#[tokio::test]
async fn test_query_by_is_a() {
    let s = make_scaffold_with_data(&["s1", "s2"]).await;
    let filter = FilterExpr::is_a("test");
    let results = s.query(&filter, &PageRequest::first(10)).await.unwrap();
    assert_eq!(results.items.len(), 2, "按 is_a=test 应返回 2 个结果");
}

/// query：limit 生效
#[tokio::test]
async fn test_query_limit() {
    let s = make_scaffold_with_data(&["l1", "l2", "l3"]).await;
    let filter = FilterExpr::is_a("test");
    let results = s.query(&filter, &PageRequest::first(2)).await.unwrap();
    assert!(results.items.len() <= 2, "limit=2 时应返回不超过 2 个结果");
}

/// query：空 store 返回空
#[tokio::test]
async fn test_query_empty_store() {
    let s = make_scaffold().await;
    let filter = FilterExpr::is_a("anything");
    let results = s.query(&filter, &PageRequest::first(10)).await.unwrap();
    assert!(results.items.is_empty(), "空 store 搜索应返回空");
}

// ─── count 测试 ───

/// count：返回正确计数
#[tokio::test]
async fn test_count_returns_correct_number() {
    let s = make_scaffold_with_data(&["x", "y"]).await;
    let n = s
        .query(&FilterExpr::match_all(), &PageRequest::first(0))
        .await
        .unwrap()
        .total;
    assert_eq!(n, 2, "count 应返回 2");
}

/// count：空 store 返回 0
#[tokio::test]
async fn test_count_empty_store() {
    let s = make_scaffold().await;
    let n = s
        .query(&FilterExpr::match_all(), &PageRequest::first(0))
        .await
        .unwrap()
        .total;
    assert_eq!(n, 0, "空 store count 应返回 0");
}

/// query 按 is_a 过滤
#[tokio::test]
async fn test_query_filtered_by_relation() {
    let s = make_scaffold_with_data(&["sf1", "sf2"]).await;
    let filter = FilterExpr::is_a("test");
    let results = s.query(&filter, &PageRequest::first(10)).await.unwrap();
    assert_eq!(results.items.len(), 2, "query 按 is_a=test 应返回 2 个结果");
}

/// query limit 生效
#[tokio::test]
async fn test_query_filtered_limit() {
    let s = make_scaffold_with_data(&["sl1", "sl2", "sl3"]).await;
    let filter = FilterExpr::is_a("test");
    let results = s.query(&filter, &PageRequest::first(2)).await.unwrap();
    assert!(
        results.items.len() <= 2,
        "limit=2 时 query 应返回不超过 2 个结果"
    );
}

// ─── snapshot cache 测试 ───

/// invalidate_snapshot_cache 后下次读取应为空
#[tokio::test]
async fn test_invalidate_snapshot_cache() {
    let s = make_scaffold_with_data(&["snap1"]).await;
    // 写入快照
    {
        let mut snap = s.snapshot_cache.write().await;
        let mut new_data = HashMap::new();
        new_data.insert("snap1".to_string(), make_test_unit_simple());
        *snap = StdArc::new(VersionedSnapshot {
            data: new_data,
            created_at: Instant::now(),
            ..Default::default()
        });
    }
    // 验证快照有数据
    {
        let snap = s.snapshot_cache.read().await;
        assert_eq!(snap.data.len(), 1);
    }
    // 失效
    s.invalidate_snapshot_cache().await;
    {
        let snap = s.snapshot_cache.read().await;
        assert!(snap.data.is_empty(), "invalidate 后快照应为空");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// update 持久化链路逐层测试
//
// 测试策略：从底层 store → get + apply_update + update 逐层验证。
// ═══════════════════════════════════════════════════════════════════════════

/// 层 1：MemoryStore.update 能正确持久化字段变更
#[tokio::test]
async fn test_memory_store_update_persists_fields() {
    let store = build_in_memory_store();

    // 插入
    let mut u = new_cu();
    u.set_id("identity");
    u.set_name("系统管家");
    u.set_description("desc");
    u.add_relation("is_a", "fact");
    u.set("level", serde_json::Value::String("msg".to_string()));
    store.insert(&u).await.unwrap();

    // 修改
    let mut updated = store.get("identity").await.unwrap().unwrap();
    updated.set_name("张三");
    updated.set("alias", serde_json::json!("系统管家"));
    store.update(&updated).await.unwrap();

    // 验证
    let stored = store.get("identity").await.unwrap().unwrap();
    assert_eq!(
        stored.name(),
        Some("张三"),
        "store.update 后 name 应为 '张三'"
    );
    assert_eq!(
        stored.get("alias").and_then(|v| v.as_str()),
        Some("系统管家"),
        "store.update 后 alias 应为 '系统管家'"
    );
}

/// 层 2：get + apply_update + update 更新路径
///
/// 模拟 memory.save 的核心逻辑：
/// 1. get(id) 获取 CognitiveUnit
/// 2. apply_update(updates) 合并更新
/// 3. update(&unit) 持久化
#[tokio::test]
async fn test_get_apply_update_update() {
    let s = make_scaffold_with_data(&["identity"]).await;

    // Step 1: get
    let mut unit = s.get("identity").await.unwrap().unwrap();

    // Step 2: apply_update
    unit.apply_update(&serde_json::json!({
        "name": "张三",
        "alias": "系统管家"
    }))
    .unwrap();

    // Step 3: update
    let result = s.update(&unit).await;
    assert!(result.is_ok(), "update 应成功: {:?}", result.err());

    // 验证
    let stored = s.store.get("identity").await.unwrap().unwrap();
    assert_eq!(stored.name(), Some("张三"), "update 后 name 应为 '张三'");
    assert_eq!(
        stored.get("alias").and_then(|v| v.as_str()),
        Some("系统管家"),
        "update 后 alias 应为 '系统管家'"
    );
}

/// 层 3：验证 get + apply_update + update 后原始字段不丢失
#[tokio::test]
async fn test_update_preserves_other_fields() {
    let s = make_scaffold_with_data(&["identity"]).await;

    let mut unit = s.get("identity").await.unwrap().unwrap();
    unit.apply_update(&serde_json::json!({"name": "张三"}))
        .unwrap();
    s.update(&unit).await.unwrap();

    // 验证：description、is_a 等原始字段仍在
    let stored = s.store.get("identity").await.unwrap().unwrap();
    assert_eq!(stored.name(), Some("张三"));
    assert!(
        stored.description().is_some() && !stored.description().unwrap().is_empty(),
        "description 不应丢失"
    );
    assert!(stored.is_type("test"), "is_a 关系不应丢失");
}

/// 层 4：模拟 memory.save op 的完整 execute 流程（局部更新模式）
#[tokio::test]
async fn test_save_update_full_flow_simulation() {
    let s = make_scaffold_with_data(&["identity"]).await;

    // ── 精确复制 memory.save 的局部更新逻辑 ──
    let target_id = "identity";
    let updates = serde_json::json!({"alias": "系统管家", "name": "张三"});

    // Step 1: engine.get(target_id)
    let mut unit = s
        .get(target_id)
        .await
        .unwrap()
        .expect("应找到 identity 单元");

    // Step 2: apply_update
    unit.apply_update(&updates).unwrap();

    // Step 3: engine.update(&unit)
    let result = s.update(&unit).await;
    assert!(result.is_ok(), "update 应成功: {:?}", result.err());

    // Step 4: 验证持久化
    let stored = s.store.get("identity").await.unwrap().unwrap();
    assert_eq!(
        stored.name(),
        Some("张三"),
        "最终 store 中 name 应为 '张三'"
    );
    assert_eq!(
        stored.get("alias").and_then(|v| v.as_str()),
        Some("系统管家"),
        "最终 store 中 alias 应为 '系统管家'"
    );

    // Step 5: 再次 get 验证变更可见
    let re_get = s.get(target_id).await.unwrap().unwrap();
    assert_eq!(
        re_get.name(),
        Some("张三"),
        "再次 get 应能看到更新后的 name"
    );
}

/// 层 5：DirStorage 文件持久化测试
///
/// 验证 store.update 写入文件后，重新从同一目录加载能读到更新。
/// 这是最贴近生产环境的测试。
#[tokio::test]
async fn test_dir_storage_update_persists_to_file() {
    use crate::plugins::agent::core::{AgentConfig, StorageBackendType};
    use crate::plugins::agent::store::build_store;

    let tmp = tempfile::tempdir().unwrap();
    let config = AgentConfig {
        storage_backend: StorageBackendType::Dir,
        ..AgentConfig::default()
    };
    let store = build_store(&config, tmp.path()).await.unwrap();

    // 插入
    let mut u = new_cu();
    u.set_id("identity");
    u.set_name("系统管家");
    u.set_description("desc");
    u.add_relation("is_a", "fact");
    u.set("level", serde_json::Value::String("msg".to_string()));
    store.insert(&u).await.unwrap();

    // 更新
    let mut updated = store.get("identity").await.unwrap().unwrap();
    updated.set_name("张三");
    updated.set("alias", serde_json::json!("系统管家"));
    store.update(&updated).await.unwrap();

    // 验证：同 store 实例能读到更新
    let stored = store.get("identity").await.unwrap().unwrap();
    assert_eq!(
        stored.name(),
        Some("张三"),
        "同实例 update 后 name 应为 '张三'"
    );

    // 验证：新建 store 实例从同一目录加载也能读到更新
    let store2 = build_store(&config, tmp.path()).await.unwrap();
    let stored2 = store2.get("identity").await.unwrap().unwrap();
    assert_eq!(stored2.name(), Some("张三"), "重新加载后 name 应为 '张三'");
    assert_eq!(
        stored2.get("alias").and_then(|v| v.as_str()),
        Some("系统管家"),
        "重新加载后 alias 应为 '系统管家'"
    );
}

/// 层 6：DirStorage 上的 update 全链路测试
#[tokio::test]
async fn test_dir_storage_update_full_flow() {
    use crate::plugins::agent::core::{AgentConfig, StorageBackendType};
    use crate::plugins::agent::store::build_store;
    use crate::plugins::agent::store::mindscape::cognitive_feedback::CognitiveFeedback;

    use crate::plugins::agent::store::mindscape::scaffold::VersionedSnapshot;

    let tmp = tempfile::tempdir().unwrap();
    let config = AgentConfig {
        storage_backend: StorageBackendType::Dir,
        ..AgentConfig::default()
    };
    let store = build_store(&config, tmp.path()).await.unwrap();

    // 插入种子单元
    let mut u = new_cu();
    u.set_id("identity");
    u.set_name("系统管家");
    u.set_description("desc");
    u.add_relation("is_a", "fact");
    u.set("level", serde_json::Value::String("msg".to_string()));
    store.insert(&u).await.unwrap();

    // 构造 scaffold
    let feedback = CognitiveFeedback::new(store.clone());

    let scaffold = MindscapeScaffold {
        store: store.clone(),

        feedback,
        snapshot_cache: tokio::sync::RwLock::new(StdArc::new(VersionedSnapshot {
            data: HashMap::new(),
            created_at: Instant::now(),
            ..Default::default()
        })),
    };

    // 模拟 update 全链路：get → apply_update → update
    let target_id = "identity";
    let updates = serde_json::json!({"alias": "系统管家", "name": "张三"});

    let mut unit = scaffold
        .get(target_id)
        .await
        .unwrap()
        .expect("应找到 identity");
    unit.apply_update(&updates).unwrap();

    let result = scaffold.update(&unit).await;
    assert!(
        result.is_ok(),
        "DirStorage update 应成功: {:?}",
        result.err()
    );

    // 验证文件持久化：新建 store 实例读取
    let store2 = build_store(&config, tmp.path()).await.unwrap();
    let stored = store2.get("identity").await.unwrap().unwrap();
    assert_eq!(
        stored.name(),
        Some("张三"),
        "DirStorage 文件持久化后 name 应为 '张三'"
    );
    assert_eq!(
        stored.get("alias").and_then(|v| v.as_str()),
        Some("系统管家"),
        "DirStorage 文件持久化后 alias 应为 '系统管家'"
    );
}
