//! memory.reflect: 反思——把对话经验提炼为持久化认知单元
//!
//! 自我进化闭环的关键一环：Agent 在完成实质性任务后调用本操作，
//! 把对话中产生的经验/洞察/规则修订写回 Mindscape，下次对话的系统
//! 提示词会自动纳入这些新认知。
//!
//! ## 设计
//!
//! 输入分为三部分：
//! - `summary`：本次对话/任务的要点摘要（写入 meta_reflection 记录）
//! - `insights`：洞察列表，每条 → 一个 experience/fact CU（priority=0，进入系统提示词最前）
//! - `rule_updates`：规则修订，带 id → 局部更新；不带 id → 新建 rule CU
//!
//! ## 与现有机制的协同
//!
//! - 复用 `AgentStore::upsert` / `get` / `update`，不引入新 trait
//! - 新 CU 的 `meta_belief` 初值由 `confidence` 字段决定（兜底 0.5）
//! - 通过 `bump_meta_belief` 实现"成功经验强化、过时认知弱化"
//! - 通过 `submit_cognition_op!` 自注册到 OpRegistry，自动出现在
//!   `agent_cognition` 工具的 operation 列表中

use serde_json::{json, Value};
use std::sync::Arc;

use crate::plugins::agent::core::{
    now_secs, types::cu_fields, AgentStore, CognitiveUnit, OperationResult,
};

pub struct ReflectOp;

#[async_trait::async_trait]
impl crate::plugins::agent::capabilities::ops::CognitionOp for ReflectOp {
    fn meta(&self) -> crate::symbio_core::CapabilityMeta {
        crate::symbio_core::CapabilityMeta {
            name: "memory.reflect".to_string(),
            description:
"反思：把对话经验提炼为持久化认知单元（CU），驱动自我进化。\n\
\n\
时机：完成实质性任务后（解决了问题/学到了新东西/发现了更好做法）主动调用。\n\
效果：经验写入后，下次对话的系统提示词会自动纳入这些新认知。\n\
\n\
参数：\n\
- summary（必填）：本次对话要点，1-2 句话。\n\
- insights（选填）：洞察数组，每条 {type, content, confidence?}。\n\
  · type 可选：experience（经验教训，默认）/ fact（事实知识）/ strategy（思维策略）\n\
  · content：具体内容\n\
  · confidence：0.0-1.0，默认 0.7\n\
- rule_updates（选填）：规则修订数组，每条 {id?, pattern, action, confidence?}。\n\
  · 带 id：更新已存在的规则 CU（局部更新 pattern/action/confidence）\n\
  · 不带 id：新建 rule CU\n\
\n\
priority 自动设置：\n\
  · 反思内容（insight/fact/rule）：`priority=0`（自动进入系统提示词最前）\n\
  · 反思日志（meta_reflection）：`priority=10`（不进入提示词，避免占用）\n\
  · 无需手动指定 priority\n\
\n\
示例：\n\
- {summary:'调试了 Rust 借用错误', insights:[{type:'experience', content:'生命周期标注要从调用处反推', confidence:0.85}]}\n\
- {summary:'发现工具调用顺序优化', rule_updates:[{pattern:'批量写文件前先检查目录', action:'避免重复创建目录', confidence:0.9}]}"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "本次对话/任务的要点摘要（1-2 句话）"
                    },
                    "insights": {
                        "type": "array",
                        "description": "洞察列表，每条提炼为一个 CU",
                        "items": {
                            "type": "object",
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "description": "认知类型：experience(默认)/fact/strategy"
                                },
                                "content": {
                                    "type": "string",
                                    "description": "具体内容"
                                },
                                "confidence": {
                                    "type": "number",
                                    "description": "置信度 0.0-1.0，默认 0.7"
                                }
                            },
                            "required": ["content"]
                        }
                    },
                    "rule_updates": {
                        "type": "array",
                        "description": "规则修订列表",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {"type": "string", "description": "已存在规则 CU 的 id（更新）；省略则新建"},
                                "pattern": {"type": "string", "description": "规则模式/触发条件"},
                                "action": {"type": "string", "description": "规则动作/应执行的行为"},
                                "confidence": {"type": "number", "description": "置信度，默认 0.8"}
                            },
                            "required": ["pattern", "action"]
                        }
                    }
                },
                "required": ["summary"],
                "additionalProperties": false
            }),
            ..Default::default()
        }
    }

    async fn execute(&self, engine: Arc<dyn AgentStore>, params: &Value) -> OperationResult {
        // 1. 必填校验：summary
        let summary = match params.get("summary").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                return OperationResult::error(
                    "缺少必需参数 'summary'。memory.reflect 需要本次对话的要点摘要。\
                     示例：{summary:'调试了借用错误', insights:[{content:'...'}]}"
                        .to_string(),
                );
            },
        };

        let mut saved_ids: Vec<String> = Vec::new();
        let mut updated_ids: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        // 2. 处理 insights → experience/fact/strategy CU
        if let Some(insights) = params.get("insights").and_then(|v| v.as_array()) {
            for (i, item) in insights.iter().enumerate() {
                match persist_insight(&engine, item).await {
                    Ok(id) => saved_ids.push(id),
                    Err(e) => errors.push(format!("insight[{i}]: {e}")),
                }
            }
        }

        // 3. 处理 rule_updates → rule CU（新建或更新）
        if let Some(rules) = params.get("rule_updates").and_then(|v| v.as_array()) {
            for (i, item) in rules.iter().enumerate() {
                match persist_rule_update(&engine, item).await {
                    Ok(PersistOutcome::Created(id)) => saved_ids.push(id),
                    Ok(PersistOutcome::Updated(id)) => updated_ids.push(id),
                    Err(e) => errors.push(format!("rule_updates[{i}]: {e}")),
                }
            }
        }

        // 4. 写入 meta_reflection 反思日志（priority=10，候选池内中等位置，可被 memory.retrieve 检索）
        let reflection_id =
            match persist_reflection_log(&engine, summary, &saved_ids, &updated_ids).await {
                Ok(id) => id,
                Err(e) => {
                    // 反思日志失败不阻断主流程，记录到 errors
                    errors.push(format!("reflection_log: {e}"));
                    String::new()
                },
            };

        OperationResult::success(json!({
            "status": "reflected",
            "summary": summary,
            "created_count": saved_ids.len(),
            "updated_count": updated_ids.len(),
            "created_ids": saved_ids,
            "updated_ids": updated_ids,
            "reflection_log_id": reflection_id,
            "errors": errors,
        }))
    }
}

/// 持久化单条 insight → CU
///
/// - type=experience → is_a=["experience"]
/// - type=fact       → is_a=["fact"]
/// - type=strategy   → is_a=["strategy"]
/// - 其它/缺失       → 默认 experience
async fn persist_insight(engine: &Arc<dyn AgentStore>, item: &Value) -> Result<String, String> {
    let content = item
        .get("content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "content 字段缺失或为空".to_string())?;

    let cu_type = item
        .get("type")
        .and_then(|v| v.as_str())
        .filter(|s| matches!(*s, "experience" | "fact" | "strategy"))
        .unwrap_or("experience");

    let confidence = item
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|f| f as f32)
        .unwrap_or(0.7)
        .clamp(0.0, 1.0);

    let mut cu = CognitiveUnit::generate_id();
    cu.set_name(content);
    cu.set_description(content);
    cu.add_type(cu_type);
    // 反思的 insight 默认就是"重要认知"，priority=0 让其强制进入系统提示词最前
    cu.set(cu_fields::PRIORITY, json!(0));
    cu.set_confidence(confidence);
    // meta_belief 初值跟随 confidence，让系统提示词预算分配器能正确评估重要性
    cu.set_meta_belief(confidence);

    let saved = engine
        .upsert(&cu)
        .await
        .map_err(|e| format!("upsert 失败: {e}"))?;
    Ok(saved.id().to_string())
}

/// 持久化单条 rule_update → 新建或更新 rule CU
async fn persist_rule_update(
    engine: &Arc<dyn AgentStore>,
    item: &Value,
) -> Result<PersistOutcome, String> {
    let pattern = item
        .get("pattern")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "pattern 字段缺失或为空".to_string())?;
    let action = item
        .get("action")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "action 字段缺失或为空".to_string())?;
    let confidence = item
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|f| f as f32)
        .unwrap_or(0.8)
        .clamp(0.0, 1.0);

    // 带 id → 更新已存在规则
    if let Some(id) = item
        .get(cu_fields::ID)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if let Ok(Some(mut existing)) = engine.get(id).await {
            existing.set_description(format!("当 {pattern} 时：{action}"));
            existing.set("rule_pattern", Value::String(pattern.to_string()));
            existing.set("rule_action", Value::String(action.to_string()));
            existing.set_confidence(confidence);
            // 规则被修订 = 被重新审视 → 提升信念度（强化有效规则）
            existing.bump_meta_belief(0.05);
            engine
                .update(&existing)
                .await
                .map_err(|e| format!("update 失败: {e}"))?;
            return Ok(PersistOutcome::Updated(id.to_string()));
        }
        // id 给了但 store 里没有 → 走新建路径（用该 id）
    }

    // 新建 rule CU
    let mut cu = if let Some(id) = item
        .get(cu_fields::ID)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        CognitiveUnit::new(id)
    } else {
        CognitiveUnit::generate_id()
    };
    cu.set_name(pattern);
    cu.set_description(format!("当 {pattern} 时：{action}"));
    cu.add_type("rule");
    // 规则强制进入提示词（priority=0）
    cu.set(cu_fields::PRIORITY, json!(0));
    cu.set_confidence(confidence);
    cu.set_meta_belief(confidence);
    cu.set("rule_pattern", Value::String(pattern.to_string()));
    cu.set("rule_action", Value::String(action.to_string()));

    let saved = cu;
    let saved = engine
        .upsert(&saved)
        .await
        .map_err(|e| format!("upsert 失败: {e}"))?;
    Ok(PersistOutcome::Created(saved.id().to_string()))
}

/// 写入 meta_reflection 反思日志（priority=10，候选池内中等位置，可被 memory.retrieve 检索）
async fn persist_reflection_log(
    engine: &Arc<dyn AgentStore>,
    summary: &str,
    created_ids: &[String],
    updated_ids: &[String],
) -> Result<String, String> {
    let mut cu = CognitiveUnit::generate_id();
    cu.set_name(format!("反思记录 {}", short_date()));
    cu.set_description(summary);
    cu.add_type("experience");
    // 标记为反思日志，便于检索时与普通经验区分
    cu.set(
        "reflection_kind",
        Value::String("meta_reflection".to_string()),
    );
    // 反思日志 priority=10（不进提示词候选，避免日志挤占预算）
    // LLM 用 memory.retrieve 检索历史反思
    cu.set(cu_fields::PRIORITY, json!(10));
    cu.set_confidence(0.6);
    cu.set_meta_belief(0.6);
    cu.set("reflected_created", json!(created_ids));
    cu.set("reflected_updated", json!(updated_ids));
    cu.set("reflected_at", json!(now_secs()));

    let saved = engine
        .upsert(&cu)
        .await
        .map_err(|e| format!("upsert 失败: {e}"))?;
    Ok(saved.id().to_string())
}

/// 新建 vs 更新的结果区分
enum PersistOutcome {
    Created(String),
    Updated(String),
}

/// 当前日期的简短表示（用于反思日志标题），格式 YYYYMMDD
fn short_date() -> String {
    use time::macros::format_description;
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc();
    let fmt = format_description!("[year][month][day]");
    now.format(&fmt).unwrap_or_else(|_| "unknown".to_string())
}

crate::submit_cognition_op!(ReflectOp);

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests;
