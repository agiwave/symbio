use super::*;
use crate::plugins::agent::core::{
    cu_from_json, new_cognitive_unit, AgentStore, FilterExpr, PageRequest,
};
use serde_json::json;
use tempfile::tempdir;

/// 辅助：count — 通过 query(match_all, first(0)) 获取 total
async fn count(store: &SqliteStorage, filter: Option<&FilterExpr>) -> usize {
    let f = filter.cloned().unwrap_or_else(FilterExpr::match_all);
    store.query(&f, &PageRequest::first(0)).await.unwrap().total
}

/// 辅助：构造测试用 storage 并插入 4 个单元
async fn setup_storage() -> (tempfile::TempDir, SqliteStorage) {
    let dir = tempdir().unwrap();
    let storage = SqliteStorage::new(dir.path());
    let cu1 = json!({"id": "rule::a", "name": "Alpha", "_ext_version": 1, "priority": 0, "is_a": ["rule"]});
    let cu2 = json!({"id": "rule::b", "name": "Beta",  "_ext_version": 1, "priority": 10, "is_a": ["rule::x", "rule::y"]});
    let cu3 = json!({"id": "fact::1", "name": "Gamma", "_ext_version": 1, "priority": 50, "is_a": ["fact"]});
    let cu4 = json!({"id": "fact::2", "name": "Delta", "_ext_version": 1, "priority": 50, "is_a": ["fact"]});
    storage.insert(&cu_from_json(cu1)).await.unwrap();
    storage.insert(&cu_from_json(cu2)).await.unwrap();
    storage.insert(&cu_from_json(cu3)).await.unwrap();
    storage.insert(&cu_from_json(cu4)).await.unwrap();
    (dir, storage)
}

#[tokio::test]
async fn test_sqlite_storage() {
    let dir = tempdir().unwrap();
    let storage = SqliteStorage::new(dir.path());

    let au = json!({
        "id": "test::1",
        "name": "Test Unit",
        "_ext_version": 1,
        "priority": 10
    });
    let cu = cu_from_json(au);

    // insert
    let inserted = storage.insert(&cu).await.unwrap();
    assert_eq!(inserted.id(), "test::1");

    // get
    let fetched = storage.get("test::1").await.unwrap().unwrap();
    assert_eq!(fetched.name(), Some("Test Unit"));

    // count
    let n = count(&storage, None).await;
    assert_eq!(n, 1);
}

#[tokio::test]
async fn test_sqlite_filter_sql_pushdown() {
    let (_dir, storage) = setup_storage().await;

    // 1) count: 全集
    let n = count(&storage, None).await;
    assert_eq!(n, 4);

    // 2) count: StartsWith (ID 前缀)
    let n = count(
        &storage,
        Some(&FilterExpr::StartsWith {
            key: "id".into(),
            prefix: "rule::".into(),
        }),
    )
    .await;
    assert_eq!(n, 2);

    // 3) count: Eq on JSON 字段
    let n = count(
        &storage,
        Some(&FilterExpr::eq(cu_fields::PRIORITY, json!(0))),
    )
    .await;
    assert_eq!(n, 1);

    // 4) count: In
    let n = count(
        &storage,
        Some(&FilterExpr::In {
            key: cu_fields::ID.into(),
            values: vec![json!("fact::1"), json!("fact::2")],
        }),
    )
    .await;
    assert_eq!(n, 2);

    // 5) query: Contains (LIKE %substring%)
    let page = storage
        .query(
            &FilterExpr::Contains {
                key: cu_fields::NAME.into(),
                substring: "am".into(),
            },
            &PageRequest::first(10),
        )
        .await
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].id(), "fact::1");

    // 6) query: Relation is_a 通配
    let page = storage
        .query(
            &FilterExpr::Relation {
                key: "is_a".into(),
                value: "rule::*".into(),
            },
            &PageRequest::first(10),
        )
        .await
        .unwrap();
    assert_eq!(page.total, 2);

    // 7) query: And 组合
    let page = storage
        .query(
            &FilterExpr::And(vec![
                FilterExpr::is_a("fact"),
                FilterExpr::eq(cu_fields::PRIORITY, json!(50)),
            ]),
            &PageRequest::first(10),
        )
        .await
        .unwrap();
    assert_eq!(page.total, 2);
}

/// 验证编译 FilterExpr 的 WHERE 子句与参数拼接正确
#[test]
fn test_compile_filter_basic_ops() {
    // Eq { key: "id" } -> id = ?（ID 过滤走通用 Eq）
    let c = compile_filter(&FilterExpr::eq("id", json!("x")));
    assert_eq!(c.where_clause, "id = ?");
    assert_eq!(c.params.len(), 1);

    // StartsWith { key: "id" } -> id LIKE 'x%'
    let c = compile_filter(&FilterExpr::StartsWith {
        key: "id".into(),
        prefix: "rule::".into(),
    });
    assert_eq!(c.where_clause, "id LIKE ?");
    assert_eq!(c.params.len(), 1);

    // Eq on JSON 字段
    let c = compile_filter(&FilterExpr::eq(cu_fields::PRIORITY, json!(0)));
    assert!(c
        .where_clause
        .contains("json_extract(data, '$.priority') = ?"));

    // In
    let c = compile_filter(&FilterExpr::In {
        key: cu_fields::ID.into(),
        values: vec![json!("a"), json!("b")],
    });
    assert!(c.where_clause.contains("IN (?,?)"));

    // And
    let c = compile_filter(&FilterExpr::and(vec![
        FilterExpr::StartsWith {
            key: "id".into(),
            prefix: "rule::".into(),
        },
        FilterExpr::eq(cu_fields::PRIORITY, json!(0)),
    ]));
    assert!(c.where_clause.starts_with("("));
    assert!(c.where_clause.ends_with(")"));
    assert!(c.where_clause.contains(" AND "));
}

/// FTS5 基础搜索（通过统一 query + Semantic filter）
#[tokio::test]
async fn test_sqlite_fts5_basic_search() {
    let tmp = std::env::temp_dir().join(format!("symbio_fts5_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let store = SqliteStorage::new(&tmp);

    let contents = [
        ("u1", "alpha", "first"),
        ("u2", "beta", "second"),
        ("u3", "gamma", "third alpha"),
        ("u4", "delta", "fourth beta"),
        ("u5", "epsilon", "fifth"),
    ];
    for (id, name, desc) in contents {
        let mut u = new_cognitive_unit();
        u.set_id(id.to_string());
        u.set_name(name);
        u.set_description(desc);
        u.set(cu_fields::PRIORITY, serde_json::json!(50));
        store.insert(&u).await.unwrap();
    }

    // 搜"alpha"：应命中 u1 + u3
    let page = store
        .query(
            &FilterExpr::Semantic {
                query: "alpha".into(),
                min_score: 0.0,
            },
            &PageRequest::first(10),
        )
        .await
        .unwrap();
    assert!(!page.items.is_empty(), "alpha 应至少命中 1 个");
    let ids: Vec<&str> = page.items.iter().map(|x| x.id()).collect();
    assert!(ids.contains(&"u1"), "u1 应被命中");
    assert!(ids.contains(&"u3"), "u3 应被命中（描述含 alpha）");
    assert!(!ids.contains(&"u5"), "u5 不含 alpha，不应被命中");
    assert!(page.scores.is_some(), "语义搜索应返回 scores");

    // 搜"beta"：应命中 u2 + u4
    let page = store
        .query(
            &FilterExpr::Semantic {
                query: "beta".into(),
                min_score: 0.0,
            },
            &PageRequest::first(10),
        )
        .await
        .unwrap();
    let ids: Vec<&str> = page.items.iter().map(|x| x.id()).collect();
    assert!(ids.contains(&"u2") && ids.contains(&"u4"));

    // 搜空字符串：应回退到全表（按 rowid DESC）
    let page = store
        .query(
            &FilterExpr::Semantic {
                query: "".into(),
                min_score: 0.0,
            },
            &PageRequest::first(10),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 5);

    let _ = std::fs::remove_dir_all(&tmp);
}

/// FTS5 + 结构化约束（Semantic + And）
#[tokio::test]
async fn test_sqlite_fts5_with_filter() {
    let tmp = std::env::temp_dir().join(format!("symbio_fts5_filter_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let store = SqliteStorage::new(&tmp);

    for (i, (id, priority)) in [("u1", 0), ("u2", 0), ("u3", 0), ("u4", 200), ("u5", 200)]
        .iter()
        .enumerate()
    {
        let mut u = new_cognitive_unit();
        u.set_id(id.to_string());
        u.set_name(format!("unit{}", i));
        u.set_description("keyword alpha everywhere");
        u.set(cu_fields::PRIORITY, serde_json::json!(priority));
        store.insert(&u).await.unwrap();
    }

    // 搜 alpha + filter priority=0：应只返回 3 个
    let filter = FilterExpr::And(vec![
        FilterExpr::Semantic {
            query: "alpha".into(),
            min_score: 0.0,
        },
        FilterExpr::eq(cu_fields::PRIORITY, serde_json::json!(0)),
    ]);
    let page = store.query(&filter, &PageRequest::first(10)).await.unwrap();
    assert_eq!(page.items.len(), 3, "filter 后应只返回 priority=0 的 3 个");
    for unit in &page.items {
        assert_eq!(unit.get(cu_fields::PRIORITY), Some(&serde_json::json!(0)));
    }

    // 搜 alpha 不加 filter：应返回 5 个
    let page = store
        .query(
            &FilterExpr::Semantic {
                query: "alpha".into(),
                min_score: 0.0,
            },
            &PageRequest::first(10),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 5);

    let _ = std::fs::remove_dir_all(&tmp);
}

/// FTS5 注入安全
#[tokio::test]
async fn test_sqlite_fts5_query_sanitization() {
    let tmp = std::env::temp_dir().join(format!("symbio_fts5_sani_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let store = SqliteStorage::new(&tmp);

    let mut u = new_cognitive_unit();
    u.set_id("u1".to_string());
    u.set_name("normal");
    u.set_description("normal content");
    u.set(cu_fields::PRIORITY, serde_json::json!(50));
    store.insert(&u).await.unwrap();

    // 注入尝试：不应报错（引号被转义为 ""），但也不应删表
    let r = store
        .query(
            &FilterExpr::Semantic {
                query: "\"; DROP TABLE units; --".into(),
                min_score: 0.0,
            },
            &PageRequest::first(10),
        )
        .await;
    assert!(r.is_ok(), "注入尝试应被静默处理而非 panic");

    // 验证表仍存在
    let cnt = count(&store, None).await;
    assert_eq!(cnt, 1, "注入不应删除表");

    let _ = std::fs::remove_dir_all(&tmp);
}
