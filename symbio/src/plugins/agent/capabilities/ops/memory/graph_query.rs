//! memory.graph_query: 图结构查询

use serde_json::Value;
use std::sync::Arc;

use crate::plugins::agent::capabilities::graph::{
    find_bridge_nodes, find_path, get_neighbors, get_subgraph, infer_relations, query_by_type,
};
use crate::plugins::agent::core::{AgentStore, OperationResult};

pub struct GraphQueryOp;

#[async_trait::async_trait]
impl crate::plugins::agent::capabilities::ops::CognitionOp for GraphQueryOp {
    fn meta(&self) -> crate::symbio_core::CapabilityMeta {
        crate::symbio_core::CapabilityMeta {
            name: "memory.graph_query".to_string(),
            // I-058 深入修复：description 改为"签名式"展示，让 LLM 像看函数签名一样看每个子操作
            description: "图结构查询与遍历推理。graph_operation 参数指定 6 个子操作之一：\n\
\n\
1. neighbors(node_id, max_hops=1, limit=50): 获取邻居节点\n\
2. path(source_id, target_id, max_depth=3): 查找两点间路径\n\
3. subgraph(node_id, max_depth=2, limit=50): 获取子图\n\
4. infer(node_id, limit=10): 推断隐含关系\n\
5. by_type(type_name, limit=50): 按类型查询\n\
6. bridges(limit=10): 查找桥接节点\n\
\n\
调用示例：{operation:\"memory.graph_query\", graph_operation:\"neighbors\", node_id:\"cu_xxx\"}".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "graph_operation": {"type": "string", "description": "子操作：neighbors/path/subgraph/infer/by_type/bridges"},
                    "node_id": {"type": "string", "description": "节点 ID（neighbors/subgraph/infer 用）"},
                    "max_hops": {"type": "integer", "description": "最大跳数，默认 1（neighbors 用）"},
                    "limit": {"type": "integer", "description": "返回数量，默认 50"},
                    "source_id": {"type": "string", "description": "起点 ID（path 用）"},
                    "target_id": {"type": "string", "description": "终点 ID（path 用）"},
                    "max_depth": {"type": "integer", "description": "最大深度（path/subgraph 用）"},
                    "relation_types": {"type": "array", "description": "关系类型过滤"},
                    "infer": {"type": "boolean", "description": "是否推断隐含关系"},
                    "type_name": {"type": "string", "description": "类型名（by_type 用）"}
                },
                "required": ["graph_operation"]
            }),
            // I-060 优化：子操作结构化展示，每个子操作一个 example
            examples: Some(vec![
                "{operation:\"memory.graph_query\", graph_operation:\"neighbors\", node_id:\"cu_xxx\"}".to_string(),
                "{operation:\"memory.graph_query\", graph_operation:\"path\", source_id:\"cu_a\", target_id:\"cu_b\", max_depth:3}".to_string(),
                "{operation:\"memory.graph_query\", graph_operation:\"subgraph\", node_id:\"cu_xxx\", max_depth:2, limit:20}".to_string(),
                "{operation:\"memory.graph_query\", graph_operation:\"infer\", node_id:\"cu_xxx\", limit:10}".to_string(),
                "{operation:\"memory.graph_query\", graph_operation:\"by_type\", type_name:\"fact\"}".to_string(),
                "{operation:\"memory.graph_query\", graph_operation:\"bridges\", limit:5}".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, engine: Arc<dyn AgentStore>, params: &Value) -> OperationResult {
        let op = params
            .get("graph_operation")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let relation_types: Option<Vec<String>> = params
            .get("relation_types")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });
        let relation_types_ref = relation_types.as_deref();

        // I-038 修复：使用语义化的子操作名称
        match op {
            "neighbors" => {
                let node_id = match params.get("node_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return OperationResult::error("neighbors 需要 node_id".to_string()),
                };
                let max_hops = params.get("max_hops").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let max_results = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
                get_neighbors(engine, node_id, relation_types_ref, max_hops, max_results).await
            }
            "path" => {
                let source = match params.get("source_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return OperationResult::error("path 需要 source_id".to_string()),
                };
                let target = match params.get("target_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return OperationResult::error("path 需要 target_id".to_string()),
                };
                let max_depth = params.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
                find_path(engine, source, target, relation_types_ref, max_depth).await
            }
            "subgraph" => {
                let node_id = match params.get("node_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return OperationResult::error("subgraph 需要 node_id".to_string()),
                };
                let max_depth = params.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
                let max_nodes = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
                get_subgraph(engine, node_id, relation_types_ref, max_depth, max_nodes).await
            }
            "infer" => {
                let node_id = match params.get("node_id").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return OperationResult::error("infer 需要 node_id".to_string()),
                };
                let max_results = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                infer_relations(engine, node_id, max_results).await
            }
            "by_type" => {
                let node_type = match params.get("type_name").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return OperationResult::error("by_type 需要 type_name".to_string()),
                };
                let max_results = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
                query_by_type(engine, node_type, max_results).await
            }
            "bridges" => {
                let max_results = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                find_bridge_nodes(engine, max_results).await
            }
            "" => OperationResult::error(
                "graph_query 缺少 graph_operation 参数。可选: neighbors, path, subgraph, infer, by_type, bridges".to_string(),
            ),
            other => OperationResult::error(format!("不支持的 graph_operation: {}", other)),
        }
    }
}

crate::submit_cognition_op!(GraphQueryOp);
