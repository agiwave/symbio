//! `agent_cognition` 统一认知能力
//!
//! 提供对认知单元（CU）的增删改查 + 语义检索 + 图关系查询。
//!
//! ## 设计原则
//!
//! - **cognition 层不感知任何具体 op 的参数细节**
//! - `CognitionRequest` 只有 `operation` + `params`（serde flatten 透传）
//! - LLM 只需知道 `operation` 字段，其他参数完全自由
//! - schema 生成、参数校验、分发执行全部由 `AgentCognitionTool` 统一管理
//! - 具体参数缺失时，由统一校验层返回标准化使用提示
//!
//! ## 当前已实现 op（5 个，全部在 memory 域）
//!
//! - `memory.save`：保存/批量保存/更新 CU（**软删除也用它**：`confidence: 0` 立即删除）
//! - `memory.retrieve`：结构化过滤 + 语义检索
//! - `memory.graph_query`：图结构查询与遍历推理
//! - `memory.reflect`：基于检索结果反思与汇总
//! - `memory.consolidate`：自动遗忘/合并/晋升（周期性后台任务）
//!
//! ## 典型工作流
//!
//! 1. **保存新知识**：`memory.save` 传 `is_a`+`description`+`confidence` 即可
//! 2. **语义检索**：`memory.retrieve` 传 `filter:{semantic:"..."}`
//! 3. **检索后更新**：先 `memory.retrieve` 拿 id，再 `memory.save` 顶层带 id 局部更新
//! 4. **图探索**：先 `memory.retrieve` 拿 id，再 `memory.graph_query` 探索邻居/路径
//! 5. **软删除**：`memory.save` 传 `{id:'cu_xxx', confidence:0}` → 立即物理删除
//!
//! ## 为什么没有 view_prompt？
//!
//! 系统提示词就是 LLM 能看到的文本，单独一个 op 让 LLM "查看"自己已经看到的
//! 内容是冗余的。系统提示词过多时，应该**主动**在提示词末尾追加"预算告警"段，
//! 让 LLM 主动调用 `memory.save` (confidence:0 软删除) / `memory.consolidate` 优化。

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plugins::agent::core::{AgentStore, OperationResult};
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::CAPABILITY_AGENT_COGNITION;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginPayload,
};

// ═══════════════════════════════════════════════════════════════════════════
// 能力实现
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) struct AgentCognitionTool {
    plugin: Arc<AgentPlugin>,
}

impl AgentCognitionTool {
    pub fn new(plugin: Arc<AgentPlugin>) -> Self {
        Self { plugin }
    }

    /// 动态生成统一的 input_schema
    ///
    /// 聚合所有已注册 op 的元数据，生成 LLM 可理解的统一 schema：
    /// - 只暴露 `operation` 字段，其他参数完全自由（`additionalProperties: true`）
    /// - operation 描述中包含每个 op 的"场景 + 参数范例"，LLM 首次调用即可正确传参
    /// - 参数类型和必填信息由各 op 的 input_schema（JSON Schema）传递
    /// - 参数缺失时由统一校验层返回标准化使用提示
    fn build_schema() -> Value {
        use serde_json::json;

        let registry = super::ops::get_registry();
        let mut op_descriptions = Vec::new();
        for (name, op) in registry.iter() {
            let m = op.meta();
            // I-052 修复：每条 op 描述精简为一行（场景 + 操作）
            // 详细参数说明由各 op 自身的 description + input_schema 负责
            op_descriptions.push(format!("- {}: {}", name, m.description));
        }
        let op_list_text = op_descriptions.join("\n");

        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": format!(
                        "必填。操作标识，格式「域.操作名」。\n\n可用操作：\n{}\n\n各操作的详细参数见其 JSON Schema（每个 op 的 input_schema）。缺失必需参数时会返回带示例的提示。",
                        op_list_text
                    )
                }
            },
            "required": ["operation"],
            "additionalProperties": true
        })
    }

    /// 统一分发：查找 op → 校验必需参数 → 执行
    ///
    /// 统一处理逻辑：
    /// 1. 未知操作 → 返回支持的操作列表
    /// 2. 必需参数缺失 → 返回标准化使用提示（参数名 + op 完整 description）
    /// 3. 正常执行 → 路由到对应 op
    async fn dispatch(
        &self,
        engine: Arc<dyn AgentStore>,
        req: &CognitionRequest,
    ) -> OperationResult {
        let registry = super::ops::get_registry();
        let op_name = &req.operation;
        let params = Value::Object(req.params.clone());

        // 1. 查找操作
        let op = match registry.get(op_name) {
            Some(op) => op,
            None => {
                return OperationResult::error(format!(
                    "未知操作: '{}'。支持的操作: {:?}",
                    op_name,
                    registry.registered_ops()
                ));
            }
        };

        // 2. 统一必需参数校验
        if let Some(hint) = Self::check_required_params(op_name, &op.meta(), &params) {
            return OperationResult::error(hint);
        }

        // 3. 执行操作
        op.execute(engine, &params).await
    }

    /// 校验必需参数是否齐全，缺失时返回标准化使用提示
    ///
    /// 提示信息包含：缺失的参数名 + 该 op 的完整 description（含使用范例）。
    /// 这样 LLM 在缺参数错误中能直接看到正确用法，无需再次试错。
    fn check_required_params(
        op_name: &str,
        meta: &CapabilityMeta,
        params: &Value,
    ) -> Option<String> {
        let required: Vec<&str> = meta
            .input_schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        if required.is_empty() {
            return None;
        }

        let missing: Vec<&str> = required
            .iter()
            .filter(|field| {
                params.get(field).is_none() || params.get(field).is_some_and(|v| v.is_null())
            })
            .copied()
            .collect();

        if missing.is_empty() {
            return None;
        }

        // I-053 修复：直接使用 op 的完整 description（含使用范例）作为提示
        // 避免依赖已移除的 format_schema_params
        Some(format!(
            "操作 '{}' 缺少必需参数: {}。\n\n该操作用法：{}",
            op_name,
            missing.join(", "),
            meta.description
        ))
    }
}

#[async_trait]
impl Capability for AgentCognitionTool {
    fn meta(&self) -> CapabilityMeta {
        // 描述策略：面向用户意图而非内部架构。
        // LLM 根据工具描述判断"什么时候该用这个工具"，
        // 所以 description 必须用用户能理解的语言描述工具管理的数据和能力。
        CapabilityMeta {
            name: CAPABILITY_AGENT_COGNITION.to_string(),
            description: "管理你的长期记忆库（CU = 认知单元）。\
典型类型：fact / rule / strategy / experience / identity / skill / working_memory 等。\n\n\
**API**：用 operation 选 op（memory.save / retrieve / delete / graph_query）。\
各 op 的具体参数见其 input_schema description；缺失必需参数时，错误信息会返回该 op 的完整用法。\
⚠️ 所有 CU 字段（含 id）必须放在 items 元素内，禁止出现在请求顶层。\n\n\
**何时调**（策略层）见系统提示词中的 自我进化 / 强制检索 规则 CU，本描述不重复。"
                .to_string(),
            input_schema: Self::build_schema(),
            category: None,
            // I-051 修复 + I-056 强化：examples 全部用 items 数组的规范形式
            examples: Some(vec![
                "保存：{operation:'memory.save', items:[{is_a:['fact'], description:'Rust所有权规则', confidence:0.9}]}".to_string(),
                "更新：{operation:'memory.save', items:[{id:'cu_xxx', name:'新名字'}]}".to_string(),
                "语义检索：{operation:'memory.retrieve', filter:{semantic:'Rust'}, limit:5}".to_string(),
                "图查询：{operation:'memory.graph_query', graph_operation:'neighbors', node_id:'cu_xxx'}".to_string(),
                "软删除：{operation:'memory.save', items:[{id:'cu_xxx', confidence:0}]}".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let req: CognitionRequest = ctx.payload()?;
        let (engine, _workdir) = self.plugin.resolve_mindscape_from_ctx(ctx.as_ref()).await?;
        let result = self.dispatch(engine, &req).await;
        Ok(PluginPayload::new(&result))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 统一请求结构体
// ═══════════════════════════════════════════════════════════════════════════

/// 统一认知请求：只关心 `operation`，其余参数通过 `serde(flatten)` 透传
///
/// 设计原则：cognition 层不感知任何具体 op 的参数细节，
/// 所有参数原样传递给 ops 层，由各 op 自行解析。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CognitionRequest {
    /// 操作标识：`domain.action`，例如 `memory.save`、`memory.update`
    pub operation: String,

    /// 所有其他参数（由各 op 自行解析）
    #[serde(flatten)]
    pub params: serde_json::Map<String, Value>,
}

// ═══════════════════════════════════════════════════════════════════════════
// 工厂 + 自注册
// ═══════════════════════════════════════════════════════════════════════════

pub fn build_cognition(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Capability> {
    let cap_ctx = crate::plugins::agent::capabilities::get_capability_context(ctx.as_ref());
    Arc::new(AgentCognitionTool::new(cap_ctx.plugin.clone())) as Arc<dyn Capability>
}

crate::submit_object_creator!(CAPABILITY_AGENT_COGNITION, build_cognition, dyn Capability);

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[path = "cognition_tests.rs"]
mod tests;
