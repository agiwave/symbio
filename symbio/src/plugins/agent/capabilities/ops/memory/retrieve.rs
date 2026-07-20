//! memory.retrieve: 统一检索（结构化过滤 + 语义搜索）
//!
//! 唯一参数：`filter`（过滤条件）+ `limit`（返回数量）。
//!
//! `filter` 是一个 JSON 对象，每个 key 映射到一种过滤条件：
//!
//! | key | 说明 | 示例 |
//! |-----|------|------|
//! | `is_a` | 按类型过滤 | `{"is_a": "fact"}` |
//! | `id` | 按 ID 获取 | `{"id": "abc123"}` |
//! | `eq` | 等值过滤 | `{"eq": {"key": "confidence", "value": 0.9}}` |
//! | `ne` | 不等过滤 | `{"ne": {"key": "level", "value": "core"}}` |
//! | `gt` / `gte` / `lt` / `lte` | 数值比较 | `{"gt": {"key": "confidence", "value": 0.5}}` |
//! | `in` | 集合包含 | `{"in": {"key": "level", "values": ["sys", "msg"]}}` |
//! | `contains` | 字符串包含 | `{"contains": {"key": "description", "substring": "Rust"}}` |
//! | `relation` | 关系过滤 | `{"relation": {"name": "causes", "value": "xxx"}}` |
//! | `semantic` | 语义搜索 | `{"semantic": "Rust 所有权规则"}` |
//! | `and` / `or` / `not` | 逻辑组合 | `{"and": [{"is_a": "fact"}, {"gt": ...}]}` |
//!
//! 组合示例：
//! - 语义搜索 + 类型过滤：`{"and": [{"semantic": "Rust"}, {"is_a": "fact"}]}`
//! - 按 ID 获取：`{"id": "abc123"}`
//! - 浏览全部：`{}` 或 `{"is_a": "fact"}`

use serde_json::{json, Value};
use std::sync::Arc;

use crate::plugins::agent::core::{now_secs, AgentStore, FilterExpr, OperationResult, PageRequest};

pub struct RetrieveOp;

#[async_trait::async_trait]
impl crate::plugins::agent::capabilities::ops::CognitionOp for RetrieveOp {
    fn meta(&self) -> crate::symbio_core::CapabilityMeta {
        crate::symbio_core::CapabilityMeta {
            name: "memory.retrieve".to_string(),
            description: "统一检索认知单元。\
                过滤条件通过 filter 对象表达，支持：is_a/id/eq/ne/gt/gte/lt/lte/in/contains/relation/semantic/and/or/not。\
                示例：{filter:{is_a:'fact'}, limit:10}、{filter:{id:'abc123'}}、{filter:{and:[{semantic:'Rust'},{is_a:'fact'}]}}"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "object",
                        "description": "过滤条件。支持：is_a/id/eq/ne/gt/gte/lt/lte/in/contains/relation/semantic/and/or/not"
                    },
                    "limit": {"type": "integer", "description": "返回数量，默认 10"}
                }
            }),
            ..Default::default()
        }
    }

    async fn execute(&self, engine: Arc<dyn AgentStore>, params: &Value) -> OperationResult {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let now = now_secs();

        let default_filter = Value::Object(serde_json::Map::new());
        let filter_val = params.get("filter").unwrap_or(&default_filter);
        let filter_obj = filter_val.as_object();

        // 解析语义搜索（如果 filter 中有 semantic key）
        let semantic_query = filter_obj
            .and_then(|o| o.get("semantic"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        // 解析结构化过滤（排除 semantic key，它由语义搜索处理）
        let structured_filter = parse_filter_excluding_semantic(filter_val);

        if let Some(query) = semantic_query {
            // 语义搜索模式：semantic 作为主检索，其余 filter 作为约束
            return semantic_search_with_filter(
                &engine,
                query,
                structured_filter.as_ref(),
                limit,
                now,
            )
            .await;
        }

        // 纯结构化过滤模式
        let filter = structured_filter.unwrap_or_else(FilterExpr::match_all);
        query_and_return(&engine, &filter, limit, now).await
    }
}

/// 语义搜索 + 结构化过滤约束
async fn semantic_search_with_filter(
    engine: &Arc<dyn AgentStore>,
    query: &str,
    constraint: Option<&FilterExpr>,
    limit: usize,
    now: u64,
) -> OperationResult {
    // 构建包含 Semantic 的 filter
    let semantic_filter = FilterExpr::Semantic {
        query: query.to_string(),
        min_score: 0.0,
    };
    let filter = match constraint {
        Some(c) => FilterExpr::And(vec![semantic_filter, c.clone()]),
        None => semantic_filter,
    };
    query_and_return(engine, &filter, limit, now).await
}

/// 解析 filter 对象为 FilterExpr（排除 semantic key）
fn parse_filter_excluding_semantic(val: &Value) -> Option<FilterExpr> {
    let obj = val.as_object()?;

    // 如果只有 semantic key，没有其他条件，返回 None
    let has_non_semantic = obj.keys().any(|k| k != "semantic");
    if !has_non_semantic {
        return None;
    }

    // 构建一个排除 semantic 的临时 Value
    let filtered: serde_json::Map<String, Value> = obj
        .iter()
        .filter(|(k, _)| k.as_str() != "semantic")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if filtered.is_empty() {
        return None;
    }

    parse_filter_expr(&Value::Object(filtered))
}

/// 解析 JSON filter 对象为 FilterExpr
///
/// 支持的格式：
/// - `{"is_a": "fact"}` → IsA
/// - `{"id": "abc123"}` → Id
/// - `{"eq": {"key": "k", "value": v}}` → Eq
/// - `{"ne": {"key": "k", "value": v}}` → Ne
/// - `{"gt": {"key": "k", "value": 0.5}}` → Gt
/// - `{"gte": {"key": "k", "value": 0.5}}` → Gte
/// - `{"lt": {"key": "k", "value": 0.5}}` → Lt
/// - `{"lte": {"key": "k", "value": 0.5}}` → Lte
/// - `{"in": {"key": "k", "values": [...]}}` → In
/// - `{"contains": {"key": "k", "substring": "..."}}` → Contains
/// - `{"relation": {"name": "causes", "value": "xxx"}}` → Relation
/// - `{"and": [...]}` → And
/// - `{"or": [...]}` → Or
/// - `{"not": {...}}` → Not
fn parse_filter_expr(val: &Value) -> Option<FilterExpr> {
    let obj = val.as_object()?;

    // 简写：{"is_a": "fact"} → Relation { key: "is_a", value: "fact" }
    if let Some(v) = obj.get("is_a") {
        if let Some(s) = v.as_str() {
            return Some(FilterExpr::is_a(s));
        }
    }

    // 简写：{"id": "abc123"} → Eq { key: "id", value: "abc123" }
    if let Some(v) = obj.get("id") {
        if let Some(s) = v.as_str() {
            return Some(FilterExpr::eq("id", Value::String(s.to_string())));
        }
    }

    // Eq
    if let Some(v) = obj.get("eq") {
        if let (Some(key), Some(value)) = (v.get("key").and_then(|k| k.as_str()), v.get("value")) {
            return Some(FilterExpr::eq(key, value.clone()));
        }
    }

    // Ne
    if let Some(v) = obj.get("ne") {
        if let (Some(key), Some(value)) = (v.get("key").and_then(|k| k.as_str()), v.get("value")) {
            return Some(FilterExpr::Ne {
                key: key.to_string(),
                value: value.clone(),
            });
        }
    }

    // Gt
    if let Some(v) = obj.get("gt") {
        if let (Some(key), Some(value)) = (
            v.get("key").and_then(|k| k.as_str()),
            v.get("value").and_then(|v| v.as_f64()),
        ) {
            return Some(FilterExpr::Gt {
                key: key.to_string(),
                value,
            });
        }
    }

    // Gte
    if let Some(v) = obj.get("gte") {
        if let (Some(key), Some(value)) = (
            v.get("key").and_then(|k| k.as_str()),
            v.get("value").and_then(|v| v.as_f64()),
        ) {
            return Some(FilterExpr::Gte {
                key: key.to_string(),
                value,
            });
        }
    }

    // Lt
    if let Some(v) = obj.get("lt") {
        if let (Some(key), Some(value)) = (
            v.get("key").and_then(|k| k.as_str()),
            v.get("value").and_then(|v| v.as_f64()),
        ) {
            return Some(FilterExpr::Lt {
                key: key.to_string(),
                value,
            });
        }
    }

    // Lte
    if let Some(v) = obj.get("lte") {
        if let (Some(key), Some(value)) = (
            v.get("key").and_then(|k| k.as_str()),
            v.get("value").and_then(|v| v.as_f64()),
        ) {
            return Some(FilterExpr::Lte {
                key: key.to_string(),
                value,
            });
        }
    }

    // In
    if let Some(v) = obj.get("in") {
        if let (Some(key), Some(values)) = (
            v.get("key").and_then(|k| k.as_str()),
            v.get("values").and_then(|v| v.as_array()),
        ) {
            return Some(FilterExpr::In {
                key: key.to_string(),
                values: values.clone(),
            });
        }
    }

    // Contains
    if let Some(v) = obj.get("contains") {
        if let (Some(key), Some(sub)) = (
            v.get("key").and_then(|k| k.as_str()),
            v.get("substring").and_then(|s| s.as_str()),
        ) {
            return Some(FilterExpr::Contains {
                key: key.to_string(),
                substring: sub.to_string(),
            });
        }
    }

    // Starts with
    if let Some(v) = obj.get("starts_with") {
        if let (Some(key), Some(prefix)) = (
            v.get("key").and_then(|k| k.as_str()),
            v.get("prefix").and_then(|s| s.as_str()),
        ) {
            return Some(FilterExpr::StartsWith {
                key: key.to_string(),
                prefix: prefix.to_string(),
            });
        }
    }

    // Relation
    if let Some(v) = obj.get("relation") {
        if let (Some(name), Some(value)) = (
            v.get("name").and_then(|n| n.as_str()),
            v.get("value").and_then(|v| v.as_str()),
        ) {
            return Some(FilterExpr::Relation {
                key: name.to_string(),
                value: value.to_string(),
            });
        }
    }

    // And
    if let Some(arr) = obj.get("and").and_then(|v| v.as_array()) {
        let exprs: Vec<FilterExpr> = arr.iter().filter_map(parse_filter_expr).collect();
        if !exprs.is_empty() {
            return Some(FilterExpr::And(exprs));
        }
    }

    // Or
    if let Some(arr) = obj.get("or").and_then(|v| v.as_array()) {
        let exprs: Vec<FilterExpr> = arr.iter().filter_map(parse_filter_expr).collect();
        if !exprs.is_empty() {
            return Some(FilterExpr::Or(exprs));
        }
    }

    // Not
    if let Some(inner) = obj.get("not") {
        if let Some(expr) = parse_filter_expr(inner) {
            return Some(FilterExpr::Not(Box::new(expr)));
        }
    }

    None
}

/// 执行查询并返回结果
async fn query_and_return(
    engine: &Arc<dyn AgentStore>,
    filter: &FilterExpr,
    limit: usize,
    now: u64,
) -> OperationResult {
    match engine.query(filter, &PageRequest::first(limit)).await {
        Ok(page) => {
            let items: Vec<Value> = page
                .items
                .iter()
                .filter(|u| !is_expired(u, now))
                .map(|u| {
                    unit_to_item(
                        u,
                        page.scores.as_ref().and_then(|s| {
                            let idx = page.items.iter().position(|p| p.id() == u.id())?;
                            s.get(idx).copied()
                        }),
                    )
                })
                .collect();
            // total 始终来自 store，不受 limit 影响
            let mut result = json!({
                "items": items,
                "total": page.total,
            });
            // 语义搜索时附加 semantic key
            if let FilterExpr::Semantic { query, .. } = filter {
                result
                    .as_object_mut()
                    .unwrap()
                    .insert("semantic".to_string(), json!(query));
            }
            OperationResult::success(result)
        },
        Err(e) => OperationResult::error(format!("查询失败: {}", e)),
    }
}

fn is_expired(unit: &crate::plugins::agent::core::CognitiveUnit, now: u64) -> bool {
    unit.get("_ext_expires_at")
        .and_then(|v| v.as_u64())
        .is_some_and(|exp| exp <= now)
}

fn unit_to_item(unit: &crate::plugins::agent::core::CognitiveUnit, score: Option<f32>) -> Value {
    let mut item = unit.to_llm_value();
    if let Some(s) = score {
        item.as_object_mut()
            .unwrap()
            .insert("score".to_string(), json!(s));
    }
    item
}

crate::submit_cognition_op!(RetrieveOp);
