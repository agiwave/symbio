//! memory.save: 保存一组认知单元（CU）
//!
//! 统一接口：调用方必须传 `items` 数组（不允许顶层简写），每个元素是一个 CU 字段集合。
//! - 有 `id` → 已存在的 CU，局部更新（仅覆盖 item 中的字段）；不存在则用该 id 新建
//! - 无 `id` → 新 CU，自动生成 id
//!
//! **软删除语义**：`confidence: 0` + 有 `id` → 立即物理删除
//! - LLM 想删除某条记忆：`{id:'cu_xxx', confidence:0}`
//! - 立即生效（不等下次 consolidate）
//! - 不再提供独立的 `memory.delete` 操作（API 更简洁：只需 save）
//!
//! 单条保存 = items 只有一个元素。批量保存 = items 有多个元素。

use serde_json::{json, Value};
use std::sync::Arc;

use crate::plugins::agent::capabilities::DEFAULT_CONFIDENCE_THRESHOLD;
use crate::plugins::agent::core::{types::cu_fields, AgentStore, CognitiveUnit, OperationResult};

pub struct SaveOp;

#[async_trait::async_trait]
impl crate::plugins::agent::capabilities::ops::CognitionOp for SaveOp {
    fn meta(&self) -> crate::symbio_core::CapabilityMeta {
        crate::symbio_core::CapabilityMeta {
            name: "memory.save".to_string(),
            description:
"保存一组认知单元 CU 集合。每个 item 的 id 字段可以为空（不传）表示新建，\n\
有 id 表示局部更新已有 CU。\n\
\n\
**`priority` 决定是否进入系统提示词**（**只在同 is_a/kind 内比较**）：\n\
- `priority: 0`     → 强制排到同 kind 内最前\n\
- `priority: 1-20`  → 进入提示词候选池（按值排序）\n\
- `priority: 21+`   → 不进入提示词（按需检索）\n\
- 不传 priority     → 默认 100（不进入系统提示词，**需 LLM 显式 opt-in**）\n\
\n\
**默认不进提示词的合理性**：多数新记忆是临时/低价值/细节类，不应自动挤占系统提示词；\n\
LLM 应**显式**给【重要】或【高频需要】的记忆设 `priority ≤ 20` 才进入候选池。\n\
\n\
**`is_a` 决定 CU 的认知类型**（用于系统提示词的分组展示）：\n\
- `identity`  - 身份信息\n\
- `fact`      - 客观事实\n\
- `rule`      - 行为规则\n\
- `experience`- 经验教训\n\
- `strategy`  - 思维策略\n\
- `skill`     - 操作技能\n\
- `judgment`  - 判断决策\n\
- `emotion`   - 情绪感受\n\
- `intuition` - 直觉判断\n\
\n\
示例：\n\
- 新建经验（立刻进提示词）：{items:[{is_a:['experience'], content:'...', confidence:0.9, priority:0}]}\n\
- 新建一般知识（默认不进入提示词）：{items:[{is_a:['fact'], content:'...', confidence:0.7}]}\n\
- 新建且需要进入提示词：{items:[{is_a:['rule'], content:'...', confidence:0.9, priority:5}]}\n\
- 更新已有 CU：{items:[{id:'cu_xxx', name:'新名字'}]}\n\
- 提权/降权：{items:[{id:'cu_xxx', priority:0}]} 或 {items:[{id:'cu_xxx', priority:200}]}\n\
- **预算优化（踢出提示词）**：{items:[{id:'cu_xxx', priority:200}]}（CU 仍保留，可通过 retrieve 调出）\n\
- **软删除（立即物理删除）**：{items:[{id:'cu_xxx', confidence:0}]}"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": {"type": "object"}
                    }
                },
                "required": ["items"],
                "additionalProperties": false
            }),
            ..Default::default()
        }
    }

    async fn execute(&self, engine: Arc<dyn AgentStore>, params: &Value) -> OperationResult {
        let Some(params_obj) = params.as_object() else {
            return OperationResult::error(
                "参数必须是 JSON 对象。memory.save 必须传 items 数组。\
                 示例：{items:[{id:'cu_xxx', name:'新名字'}]}"
                    .to_string(),
            );
        };

        // 严格校验：拒绝顶层 CU 字段（包括 id）
        // 这些字段必须放在 items 数组的每个元素里
        if let Some(offending) = top_level_cu_field(params_obj) {
            return OperationResult::error(format!(
                "参数错误：'{}' 是 CU 字段，必须放在 items 数组的每个元素内，不能在请求顶层。\
                 \n正确示例：{{items:[{{id:'cu_xxx', name:'新名字'}}]}}\
                 \n错误示例：{{{}:..., items:[...]}}",
                offending, offending
            ));
        }

        // 必传 items 数组
        let items_arr = match params_obj.get("items").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr,
            Some(_) => {
                return OperationResult::error(
                    "参数 'items' 数组不能为空。memory.save 必须传非空 items 数组，\
                     每个元素是一个 CU。\
                     示例：{items:[{id:'cu_xxx', name:'新名字'}]}"
                        .to_string(),
                );
            }
            None => {
                return OperationResult::error(
                    "缺少必需参数 'items'。memory.save 必须传 items 数组格式，\
                     不支持顶层简写。\
                     示例：{items:[{id:'cu_xxx', name:'新名字'}]}"
                        .to_string(),
                );
            }
        };

        let mut results = Vec::new();
        for item in items_arr {
            let result = save_one_cu(&engine, item).await;
            match result {
                Ok(info) => results.push(info),
                Err(e) => return OperationResult::error(e),
            }
        }

        if results.len() == 1 {
            OperationResult::success(results.into_iter().next().unwrap())
        } else {
            OperationResult::success(json!({
                "items": results,
                "count": results.len(),
            }))
        }
    }
}

/// 保存单个认知单元
///
/// - 有 `id` 且 store 中已存在 → 局部更新（apply_update）
/// - 有 `id` 但 store 中不存在 → 新建
/// - 无 `id` → 新建（自动生成 id）
async fn save_one_cu(engine: &Arc<dyn AgentStore>, item: &Value) -> Result<Value, String> {
    let obj = item
        .as_object()
        .ok_or_else(|| "每个 item 必须是 JSON 对象".to_string())?;

    // 分离 id 和其余字段
    let has_id = obj.contains_key(cu_fields::ID);
    let id_str = obj
        .get(cu_fields::ID)
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // **立即软删除信号**：LLM 显式传 confidence=0 + 有 id
    // → 立即物理删除（不等下次 consolidate）
    // 这是"立即遗忘"的标准方式（也保留 consolidate 的批量清理作为补充）
    if has_id && !id_str.is_empty() {
        if let Some(conf) = obj.get(cu_fields::CONFIDENCE).and_then(|v| v.as_f64()) {
            if conf <= 0.0 {
                match engine.delete(id_str).await {
                    Ok(_) => {
                        return Ok(json!({
                            "id": id_str,
                            "action": "deleted",
                            "message": "confidence=0 → 立即删除（不等 consolidate）",
                        }))
                    }
                    Err(e) => return Err(format!("软删除失败: {}", e)),
                }
            }
        }
    }

    if has_id && !id_str.is_empty() {
        // 有 id：尝试局部更新
        if let Ok(Some(mut existing)) = engine.get(id_str).await {
            // 应用更新（apply_update 会跳过 id 和 _ext_ 字段）
            if let Err(e) = existing.apply_update(item) {
                return Err(format!("应用更新失败: {}", e));
            }
            // 软删除信号：LLM 显式传 confidence=0 → 保留为 0（不强制设默认值）
            // 仅当 confidence 字段完全未设置时才补默认值
            if existing.get(cu_fields::CONFIDENCE).is_none() {
                existing.set_confidence(DEFAULT_CONFIDENCE_THRESHOLD);
            }
            if let Err(e) = engine.update(&existing).await {
                return Err(format!("更新失败: {}", e));
            }
            return Ok(json!({
                "id": id_str,
                "action": "updated",
                "updated_fields": obj.keys()
                    .filter(|k| *k != cu_fields::ID && !k.starts_with("_ext_"))
                    .collect::<Vec<_>>(),
            }));
        }
    }

    // 新建 CU
    let cu = cu_from_item(item);
    match engine.upsert(&cu).await {
        Ok(saved) => Ok(json!({
            "id": saved.id(),
            "action": if has_id { "created_with_id" } else { "created" },
        })),
        Err(e) => Err(format!("保存失败: {}", e)),
    }
}

/// 从 JSON 对象构建 CognitiveUnit
///
/// priority 语义（**只在同 is_a/kind 内比较**）：
/// - 不传 priority → 默认 10（候选池内居中）
/// - LLM 想让新 CU 进入系统提示词：设 `priority <= 20`（阈值）
/// - LLM 想让新 CU 排最前：设 `priority=0`
/// - LLM 想让新 CU 不进入提示词：设 `priority > 20`
///
/// 单一 `priority` 维度决定"是否进入提示词"和"在同 kind 内的排序"。
fn cu_from_item(item: &Value) -> CognitiveUnit {
    let obj = item.as_object();

    let mut cu = if let Some(id) = obj
        .and_then(|o| o.get(cu_fields::ID))
        .and_then(|v| v.as_str())
    {
        if !id.is_empty() {
            CognitiveUnit::new(id)
        } else {
            CognitiveUnit::generate_id()
        }
    } else {
        CognitiveUnit::generate_id()
    };

    // 遍历所有字段，直接设置到 CU（跳过 id，它已在构造时处理）
    if let Some(obj) = obj {
        for (key, value) in obj {
            if key == cu_fields::ID {
                continue;
            }
            cu.set(key, value.clone());
        }
    }

    // 默认 priority=100：默认不进入系统提示词（opt-out）
    // 多数新记忆是临时/低价值/细节，不应自动挤占系统提示词
    // LLM 显式设 priority≤20 才进入候选池（opt-in）
    if cu.get_number(cu_fields::PRIORITY).is_none() {
        cu.set(cu_fields::PRIORITY.to_string(), json!(100));
    }

    // confidence 缺省时补 0.7（默认值）
    // **软删除信号**：LLM 显式传 confidence=0 → 保留为 0（不强制设默认值）
    if cu.get(cu_fields::CONFIDENCE).is_none() {
        cu.set_confidence(DEFAULT_CONFIDENCE_THRESHOLD);
    }

    cu
}

crate::submit_cognition_op!(SaveOp);

// ═══════════════════════════════════════════════════════════════════════════
// 严格校验：顶层 CU 字段检测
// ═══════════════════════════════════════════════════════════════════════════

/// CU 字段名集合（必须放在 items 数组内，不能在请求顶层）
const CU_FIELDS: &[&str] = &[
    cu_fields::ID,
    cu_fields::NAME,
    cu_fields::DESCRIPTION,
    cu_fields::CONTENT,
    cu_fields::CONFIDENCE,
    cu_fields::IS_A,
    cu_fields::TAGS,
    cu_fields::PRIORITY,
];

/// 检查 params 顶层是否包含任何 CU 字段，若有则返回第一个违规字段名
///
/// 设计：严格拒绝，而非自动修复。
/// 单一调用格式（{items: [...]}）让 LLM 不需要思考"哪个字段放哪"。
/// 任何 CU 字段出现在顶层都立即报错，并给出正确示例。
fn top_level_cu_field(params_obj: &serde_json::Map<String, Value>) -> Option<&'static str> {
    for field in CU_FIELDS {
        if params_obj.contains_key(*field) {
            return Some(*field);
        }
    }
    None
}

#[cfg(test)]
mod tests;
