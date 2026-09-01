//! 知识图谱最小查询实现
//!
//! # 设计动机
//!
//! v9 关系机制化之后，`causes` / `depends` / `opposite` / 自定义关系 CU 已可结构化表达，
//! 但 `agent_memory.graph_query` 长期是空桩，LLM schema 中却向其暴露了 op 类型，
//! 造成"接口承诺 vs 实际实现"不一致。
//!
//! 本模块提供 6 个最小可用图操作，全部基于 v9 关系机制化（`CognitiveUnit::relations`）
//! + v9 通用过滤（`FilterExpr::Relation`）实现，**不引入新的存储后端**。
//!
//! # 限制
//!
//! - 每次查询都通过 `engine.search` 拉取候选集合后做内存 BFS；
//   **不预构建邻接表**，节点规模 > 1000 时性能显著下降。
//! - 暂不支持**加权边**（`causes` 与 `depends` 都视为无权无向边）。
//! - 路径查询使用 BFS 求最短路径（边数最少），不保证语义相关度最优。
//!
//! # 后续演进
//!
//! - 引入"邻接表缓存"（由 `EmbeddingStore` 装饰器在后台维护）
//! - 引入"带权边 + PageRank"评估桥接节点重要性

use crate::plugins::agent::core::types::cu_fields;
use crate::plugins::agent::core::AgentStore;
use crate::plugins::agent::core::OperationResult;
use crate::plugins::agent::core::{CognitiveUnit, FilterExpr, PageRequest};
use serde_json::{json, Value};
use std::sync::Arc;

/// 图查询支持的关系类型集合
///
/// 这些名字来自 COGNITION.md §2.4 核心关系列表（prop CU 声明），
/// 运行时可通过新增 prop CU 扩展更多关系。
const TRAVERSABLE_RELATIONS: &[&str] = &[
    "is_a", "causes", "depends", "has", "part_of", "related", "similar", "opposite",
];

/// 获取节点的直接邻居（出/入边合一，按 `relation` 过滤）
///
/// # 算法
///
/// 1. 拉取所有候选 CU（按 `is_a`/`kind` 过滤后的全集，规模 N）
/// 2. 邻接关系在内存 BFS 中按 `relation_types` 过滤
/// 3. 返回 1 跳邻居 ID 列表 + 对应关系
///
/// # 参数
///
/// - `node_id`：起始节点 ID
/// - `relation_types`：限定只遍历这些关系名（None 视为全部 `TRAVERSABLE_RELATIONS`）
/// - `max_hops`：最大跳数（默认 1）
/// - `max_results`：返回结果上限（默认 50）
pub async fn get_neighbors(
    engine: Arc<dyn AgentStore>,
    node_id: &str,
    relation_types: Option<&[String]>,
    max_hops: usize,
    max_results: usize,
) -> OperationResult {
    if node_id.is_empty() {
        return OperationResult::error("node_id is required".to_string());
    }
    let max_hops = max_hops.max(1);

    let all_units = match fetch_all_units(&engine).await {
        Ok(u) => u,
        Err(e) => return e,
    };

    let allowed: Vec<String> = match relation_types {
        Some(rt) if !rt.is_empty() => rt.to_vec(),
        _ => TRAVERSABLE_RELATIONS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };

    // 邻接表：node_id -> Vec<(target_id, relation)>
    let adj = build_adjacency(&all_units, &allowed);

    // BFS
    let mut visited = std::collections::HashSet::new();
    visited.insert(node_id.to_string());
    let mut frontier = vec![(node_id.to_string(), 0usize)];
    let mut neighbors: Vec<Value> = Vec::new();

    while let Some((current, depth)) = frontier.pop() {
        if depth >= max_hops {
            continue;
        }
        if let Some(edges) = adj.get(&current) {
            for (target, relation) in edges {
                if !visited.insert(target.clone()) {
                    continue;
                }
                let depth_next = depth + 1;
                neighbors.push(json!({
                    "node_id": target,
                    "relation": relation,
                    "hop": depth_next,
                }));
                frontier.push((target.clone(), depth_next));
                if neighbors.len() >= max_results {
                    break;
                }
            }
        }
        if neighbors.len() >= max_results {
            break;
        }
    }

    OperationResult::success(json!({
        "node_id": node_id,
        "neighbors": neighbors,
        "total": neighbors.len(),
        "max_hops": max_hops,
    }))
}

/// 查找两个节点之间的最短路径（基于边数）
///
/// 返回从 `source` 到 `target` 经过的节点序列（不含 source）。
/// 若不可达，返回空路径并标注 `reachable: false`。
pub async fn find_path(
    engine: Arc<dyn AgentStore>,
    source_id: &str,
    target_id: &str,
    relation_types: Option<&[String]>,
    max_depth: usize,
) -> OperationResult {
    if source_id.is_empty() || target_id.is_empty() {
        return OperationResult::error("source_id and target_id are both required".to_string());
    }
    if source_id == target_id {
        return OperationResult::success(json!({
            "source": source_id,
            "target": target_id,
            "path": [],
            "length": 0,
            "reachable": true,
        }));
    }
    // v29-N4: clamp(1, 10) 替代 max().min()（clippy::manual_clamp）
    let max_depth = max_depth.clamp(1, 10); // 安全上限

    let all_units = match fetch_all_units(&engine).await {
        Ok(u) => u,
        Err(e) => return e,
    };

    let allowed: Vec<String> = match relation_types {
        Some(rt) if !rt.is_empty() => rt.to_vec(),
        _ => TRAVERSABLE_RELATIONS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };

    let adj = build_adjacency(&all_units, &allowed);

    // BFS 找最短路径
    let mut visited = std::collections::HashMap::new();
    visited.insert(source_id.to_string(), None::<String>);
    let mut frontier = vec![source_id.to_string()];

    let mut found = false;
    'outer: while let Some(current) = frontier.pop() {
        if let Some(parent_chain) = reconstruct_depth(&visited, &current) {
            if parent_chain.len() > max_depth {
                continue;
            }
        }
        if let Some(edges) = adj.get(&current) {
            for (target, _relation) in edges {
                if visited.contains_key(target) {
                    continue;
                }
                visited.insert(target.clone(), Some(current.clone()));
                if target == target_id {
                    found = true;
                    break 'outer;
                }
                frontier.push(target.clone());
            }
        }
    }

    if !found {
        return OperationResult::success(json!({
            "source": source_id,
            "target": target_id,
            "path": [],
            "length": 0,
            "reachable": false,
        }));
    }

    // 回溯路径
    let mut path = vec![target_id.to_string()];
    let mut cur = target_id.to_string();
    // v29-N4: 去掉 `.map(|v| v)`（clippy::unnecessary_map）
    while let Some(Some(parent)) = visited.get(&cur).cloned() {
        path.push(parent.clone());
        cur = parent;
        if cur == source_id {
            break;
        }
    }
    path.reverse();
    path.remove(0); // 移除 source，仅返回中间节点 + target

    OperationResult::success(json!({
        "source": source_id,
        "target": target_id,
        "path": path,
        "length": path.len(),
        "reachable": true,
    }))
}

/// 获取节点 N 跳内的子图（节点 + 边）
pub async fn get_subgraph(
    engine: Arc<dyn AgentStore>,
    node_id: &str,
    relation_types: Option<&[String]>,
    max_depth: usize,
    max_nodes: usize,
) -> OperationResult {
    if node_id.is_empty() {
        return OperationResult::error("node_id is required".to_string());
    }
    let max_depth = max_depth.max(1);
    // v29-N4: clamp(1, 500) 替代 max().min()（clippy::manual_clamp）
    let max_nodes = max_nodes.clamp(1, 500);

    let all_units = match fetch_all_units(&engine).await {
        Ok(u) => u,
        Err(e) => return e,
    };

    let allowed: Vec<String> = match relation_types {
        Some(rt) if !rt.is_empty() => rt.to_vec(),
        _ => TRAVERSABLE_RELATIONS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };

    let adj = build_adjacency(&all_units, &allowed);

    let mut visited = std::collections::HashSet::new();
    visited.insert(node_id.to_string());
    let mut frontier = vec![(node_id.to_string(), 0usize)];
    let mut nodes = vec![json!({"id": node_id, "depth": 0})];
    let mut edges: Vec<Value> = Vec::new();

    while let Some((current, depth)) = frontier.pop() {
        if depth >= max_depth || nodes.len() >= max_nodes {
            continue;
        }
        if let Some(neighbors) = adj.get(&current) {
            for (target, relation) in neighbors {
                let is_new = visited.insert(target.clone());
                if is_new {
                    nodes.push(json!({"id": target, "depth": depth + 1}));
                    frontier.push((target.clone(), depth + 1));
                }
                edges.push(json!({
                    "from": current,
                    "to": target,
                    "relation": relation,
                }));
                if nodes.len() >= max_nodes {
                    break;
                }
            }
        }
    }

    OperationResult::success(json!({
        "root": node_id,
        "nodes": nodes,
        "edges": edges,
        "node_count": nodes.len(),
        "edge_count": edges.len(),
    }))
}

/// 推断节点之间的关系（基于共享邻居）
///
/// 对 `node_id` 的所有 1 跳邻居，统计与 `target_id` 的共同邻居数。
/// 共同邻居越多 → 推断关系越强。
pub async fn infer_relations(
    engine: Arc<dyn AgentStore>,
    node_id: &str,
    max_results: usize,
) -> OperationResult {
    if node_id.is_empty() {
        return OperationResult::error("node_id is required".to_string());
    }

    let all_units = match fetch_all_units(&engine).await {
        Ok(u) => u,
        Err(e) => return e,
    };

    let allowed: Vec<String> = TRAVERSABLE_RELATIONS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let adj = build_adjacency(&all_units, &allowed);

    let Some(node_neighbors) = adj.get(node_id) else {
        return OperationResult::success(json!({
            "node_id": node_id,
            "inferred": [],
            "message": "node has no traversable relations",
        }));
    };
    let node_neighbor_set: std::collections::HashSet<&str> =
        node_neighbors.iter().map(|(t, _)| t.as_str()).collect();

    // 对每个其他节点，统计共同邻居
    let mut candidates: Vec<(String, usize)> = Vec::new();
    for (other, edges) in &adj {
        if other == node_id {
            continue;
        }
        let shared = edges
            .iter()
            .filter(|(t, _)| node_neighbor_set.contains(t.as_str()))
            .count();
        if shared > 0 {
            candidates.push((other.clone(), shared));
        }
    }
    candidates.sort_by_key(|b| std::cmp::Reverse(b.1));
    // v29-N4: clamp(1, 50) 替代 max().min()（clippy::manual_clamp）
    candidates.truncate(max_results.clamp(1, 50));

    let inferred: Vec<Value> = candidates
        .into_iter()
        .map(|(id, shared)| {
            json!({
                "node_id": id,
                "shared_neighbors": shared,
            })
        })
        .collect();

    OperationResult::success(json!({
        "node_id": node_id,
        "inferred": inferred,
        "total": inferred.len(),
    }))
}

/// 查询指定类型的所有节点
pub async fn query_by_type(
    engine: Arc<dyn AgentStore>,
    node_type: &str,
    max_results: usize,
) -> OperationResult {
    if node_type.is_empty() {
        return OperationResult::error("node_type is required".to_string());
    }
    // v29-N4: clamp(1, 500) 替代 max().min()（clippy::manual_clamp）
    let max_results = max_results.clamp(1, 500);

    let filter = FilterExpr::is_a(node_type);
    let results = engine
        .query(&filter, &PageRequest::first(max_results))
        .await;

    let nodes: Vec<Value> = match results {
        Ok(page) => page
            .items
            .iter()
            .map(|u| {
                json!({
                    "id": u.id(),
                    "is_a": u.get(cu_fields::IS_A).and_then(|v| v.as_str()).unwrap_or(node_type),
                    "description": u.description().unwrap_or(""),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    OperationResult::success(json!({
        "node_type": node_type,
        "nodes": nodes,
        "total": nodes.len(),
    }))
}

/// 查找桥接节点（连接两个不连通子图的节点）
///
/// 实现：对每个节点 v，检查移除 v 后图是否分裂为更多连通分量；
/// v 的"桥接度"= 移除后增加的连通分量数。
/// 简化实现：v 的度数 - 1（高估但足够启发式）。
pub async fn find_bridge_nodes(engine: Arc<dyn AgentStore>, max_results: usize) -> OperationResult {
    let all_units = match fetch_all_units(&engine).await {
        Ok(u) => u,
        Err(e) => return e,
    };

    let allowed: Vec<String> = TRAVERSABLE_RELATIONS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let adj = build_adjacency(&all_units, &allowed);

    // 简化启发式：度数 - 1 视为"桥接度"
    let mut bridges: Vec<(String, usize)> = adj
        .iter()
        .map(|(id, edges)| (id.clone(), edges.len().saturating_sub(1)))
        .filter(|(_, score)| *score > 0)
        .collect();
    bridges.sort_by_key(|b| std::cmp::Reverse(b.1));
    // v29-N4: clamp(1, 50) 替代 max().min()（clippy::manual_clamp）
    bridges.truncate(max_results.clamp(1, 50));

    let nodes: Vec<Value> = bridges
        .into_iter()
        .map(|(id, score)| json!({"node_id": id, "bridge_score": score}))
        .collect();

    OperationResult::success(json!({
        "bridge_nodes": nodes,
        "total": nodes.len(),
        "note": "bridge_score = degree - 1 (heuristic)",
    }))
}

// 内部辅助

/// 通过 engine 分页拉取所有 CU（不区分类型）
///
/// 使用 `query` + `PageRequest` 分批加载（每批 500 条），
/// 避免单次加载全部导致内存峰值。
/// 总上限仍为 5000 条防止无限增长。
async fn fetch_all_units(
    engine: &Arc<dyn AgentStore>,
) -> Result<Vec<CognitiveUnit>, OperationResult> {
    const PAGE_SIZE: usize = 500;
    const MAX_UNITS: usize = 5000;
    let filter = FilterExpr::match_all();
    let mut all_units: Vec<CognitiveUnit> = Vec::new();
    let mut offset = 0;

    loop {
        let page = engine
            .query(&filter, &PageRequest::new(offset, PAGE_SIZE))
            .await;
        match page {
            Ok(result) => {
                let count = result.items.len();
                all_units.extend(result.items);
                if count < PAGE_SIZE || all_units.len() >= MAX_UNITS {
                    break;
                }
                offset += PAGE_SIZE;
            }
            Err(_) => break,
        }
    }

    if all_units.len() >= MAX_UNITS {
        all_units.truncate(MAX_UNITS);
        crate::plugin_warn!(
            "agent",
            "[Graph] fetch_all_units hit cap {}, results may be truncated",
            MAX_UNITS
        );
    }
    Ok(all_units)
}

/// 从 CU 集合构建邻接表（id -> Vec<(target_id, relation)>）
///
/// 支持的关系名：TRAVERSABLE_RELATIONS 集合。调用方通过 `relation_types` 参数控制。
fn build_adjacency(
    units: &[CognitiveUnit],
    allowed_relations: &[String],
) -> std::collections::HashMap<String, Vec<(String, String)>> {
    let mut adj: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();

    for unit in units {
        let id = unit.id();
        for rel in allowed_relations {
            if let Some(targets) = unit.get(rel).and_then(|v| v.as_array()) {
                for t in targets {
                    if let Some(tid) = t.as_str() {
                        adj.entry(id.to_string())
                            .or_default()
                            .push((tid.to_string(), rel.clone()));
                        // 无向图：反向也加一条
                        adj.entry(tid.to_string())
                            .or_default()
                            .push((id.to_string(), rel.clone()));
                    }
                }
            }
        }
    }
    adj
}

/// 重建 BFS 深度链（用于判断是否超过 max_depth）
fn reconstruct_depth(
    visited: &std::collections::HashMap<String, Option<String>>,
    end: &str,
) -> Option<Vec<String>> {
    let mut chain = vec![end.to_string()];
    let mut cur = end.to_string();
    while let Some(Some(parent)) = visited.get(&cur) {
        chain.push(parent.clone());
        cur = parent.clone();
    }
    Some(chain)
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
