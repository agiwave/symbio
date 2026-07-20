//! memory.consolidate: 认知整合——遗忘衰减 + 冗余合并 + 优先级晋升 + 候选池健康
//!
//! 认知卫生的核心操作：随着 Agent 的认知单元不断积累，需要定期整合以防
//! 信噪比下降。
//!
//! ## 四大机制
//!
//! 1. **遗忘衰减**：对 `priority > ENTER_PROMPT_THRESHOLD`（默认 20）的 CU，
//!    按艾宾浩斯曲线计算保留度。
//!    `retention = meta_belief × exp(-Δt_days / memory_strength)`
//!    低于阈值的 CU 被标记为"待遗忘"（dry_run 时只报告，不执行）。
//!    **保护规则**：`priority <= 20` 的 CU（候选池内）不被遗忘。
//!
//! 2. **冗余合并**：对同 is_a 类型且有 embedding 的 CU，计算两两语义相似度。
//!    相似度 > 阈值的 CU 对，保留 belief 更高者，另一条的 access_count 累加
//!    到保留者后删除。
//!
//! 3. **优先级晋升**：access_count > 阈值 且 priority > 10 的 CU，
//!    自动降低 priority（数值越小越靠前），使其在系统提示词中更易展示。
//!
//! 4. **候选池健康报告**（v54 新增，只报告不越权）：
//!    扫描 `priority <= ENTER_PROMPT_THRESHOLD` 的候选池 CU，报告以下事实：
//!      - 信念度偏低（`meta_belief < 0.3`）
//!      - 长期未被访问（`last_access` 超过 30 天）
//!        **绝不自动修改 priority 或触发删除**——价值判断完全由 LLM 自主决定。
//!        LLM 看到报告后可自主调用 `memory.save` 调整。
//!
//! ## 安全设计
//!
//! - 默认 `dry_run=true`：只返回报告，不执行任何写操作
//! - 显式 `dry_run=false` 才真正执行
//! - 候选池健康报告在任何模式下都只报告，不会触发任何写操作
//!
//! ## 与系统提示词的协同
//!
//! consolidate 执行后，被保留/晋升的 CU 自动在下次对话的系统提示词中
//! 获得更高的预算分配分数（因为 meta_belief/priority/access_count 都变化了）。

use serde_json::{json, Value};
use std::sync::Arc;

use crate::plugins::agent::core::{
    cosine_similarity, now_secs, types::cu_fields, AgentStore, CognitiveUnit, FilterExpr,
    OperationResult, PageRequest,
};
use crate::plugins::agent::handlers::system_prompt::ENTER_PROMPT_THRESHOLD;

pub struct ConsolidateOp;

// ── 阈值常量 ──

/// 遗忘曲线：保留度低于此值则标记为待遗忘
const FORGET_THRESHOLD: f64 = 0.1;
/// 冗余合并：语义相似度高于此值则合并
const MERGE_SIMILARITY_THRESHOLD: f32 = 0.92;
/// 优先级晋升：access_count > 此值的 CU 才参与晋升
const PROMOTION_ACCESS_COUNT: u64 = 10;
/// 优先级晋升：priority > 此值的 CU 才参与晋升（已靠前的不需要再升）
const PROMOTION_PRIORITY_CEILING: usize = 10;
/// 优先级晋升步长（priority 减小的量，数值越小越靠前）
const PROMOTION_STEP: usize = 5;

#[async_trait::async_trait]
impl crate::plugins::agent::capabilities::ops::CognitionOp for ConsolidateOp {
    fn meta(&self) -> crate::symbio_core::CapabilityMeta {
        crate::symbio_core::CapabilityMeta {
            name: "memory.consolidate".to_string(),
            description: "认知整合：清理冗余、遗忘过时、晋升高频认知。\n\
\n\
效果：整合后系统提示词的信噪比自动提升（高分 CU 更易被展示）。\n\
安全：默认 dry_run=true 只返回报告，不执行任何写操作。\n\
\n\
参数：\n\
- dry_run（选填）：true=只报告不执行（默认）；false=真正执行\n\
- forget_threshold（选填）：遗忘阈值 0.0-1.0，默认 0.1\n\
- merge_threshold（选填）：冗余合并相似度阈值 0.0-1.0，默认 0.92\n\
\n\
示例：\n\
- {dry_run:true}  查看整合报告\n\
- {dry_run:false} 执行整合\n\
- {dry_run:false, forget_threshold:0.05} 更激进的遗忘"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dry_run": {
                        "type": "boolean",
                        "description": "true=只报告不执行（默认）；false=执行"
                    },
                    "forget_threshold": {
                        "type": "number",
                        "description": "遗忘阈值 0.0-1.0，低于此值的 CU 被遗忘。默认 0.1"
                    },
                    "merge_threshold": {
                        "type": "number",
                        "description": "冗余合并相似度阈值 0.0-1.0，默认 0.92"
                    }
                },
                "additionalProperties": false
            }),
            ..Default::default()
        }
    }

    async fn execute(&self, engine: Arc<dyn AgentStore>, params: &Value) -> OperationResult {
        let dry_run = params
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let forget_threshold = params
            .get("forget_threshold")
            .and_then(|v| v.as_f64())
            .unwrap_or(FORGET_THRESHOLD) as f32;
        let merge_threshold = params
            .get("merge_threshold")
            .and_then(|v| v.as_f64())
            .unwrap_or(MERGE_SIMILARITY_THRESHOLD as f64) as f32;

        let now = now_secs();

        // 获取所有 CU（排除 core 级本体，它们不可被遗忘/删除）
        let all_units = match fetch_all_non_schema_units(&engine).await {
            Ok(units) => units,
            Err(e) => return OperationResult::error(format!("获取 CU 失败: {e}")),
        };

        // 1. 遗忘衰减分析
        let forget_report = analyze_forget_decay(&all_units, now, forget_threshold);

        // 2. 冗余合并分析
        let merge_report = analyze_redundancy(&all_units, merge_threshold);

        // 3. 优先级晋升分析
        let promotion_report = analyze_priority_promotion(&all_units);

        // 4. 候选池健康报告（只报告，不执行任何写操作）
        let pool_health = analyze_candidate_pool_health(&all_units, now);

        if !dry_run {
            // 执行遗忘
            let forgotten = apply_forget(&engine, &forget_report.to_forget_ids).await;
            // 执行合并
            let merged = apply_merge(&engine, &merge_report.merges).await;
            // 执行晋升
            let promoted = apply_promotion(&engine, &promotion_report.to_promote).await;

            OperationResult::success(json!({
                "status": "consolidated",
                "dry_run": false,
                "forgotten_count": forgotten,
                "forgotten_ids": forget_report.to_forget_ids,
                "merged_count": merged,
                "merge_details": merge_report.merges.iter().map(|m| json!({
                    "kept_id": m.keep_id,
                    "removed_id": m.remove_id,
                    "similarity": m.similarity,
                })).collect::<Vec<_>>(),
                "promoted_count": promoted,
                "promoted_ids": promotion_report.to_promote.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
                "candidate_pool_health": build_pool_health_json(&pool_health),
                "stats": {
                    "total_units": all_units.len(),
                    "analyzed_for_forget": forget_report.analyzed_count,
                    "forget_candidates": forget_report.to_forget_ids.len(),
                    "analyzed_for_merge": merge_report.analyzed_count,
                    "merge_candidates": merge_report.merges.len(),
                    "analyzed_for_promotion": promotion_report.analyzed_count,
                    "promotion_candidates": promotion_report.to_promote.len(),
                    "candidate_pool_size": pool_health.pool_size,
                }
            }))
        } else {
            OperationResult::success(json!({
                "status": "dry_run",
                "dry_run": true,
                "would_forget": {
                    "count": forget_report.to_forget_ids.len(),
                    "ids": forget_report.to_forget_ids,
                    "details": forget_report.details.iter().map(|d| json!({
                        "id": d.id,
                        "retention": d.retention,
                    })).collect::<Vec<_>>(),
                },
                "would_merge": {
                    "count": merge_report.merges.len(),
                    "details": merge_report.merges.iter().map(|m| json!({
                        "keep_id": m.keep_id,
                        "keep_name": m.keep_name,
                        "remove_id": m.remove_id,
                        "remove_name": m.remove_name,
                        "similarity": m.similarity,
                    })).collect::<Vec<_>>(),
                },
                "would_promote": {
                    "count": promotion_report.to_promote.len(),
                    "details": promotion_report.to_promote.iter().map(|p| json!({
                        "id": p.id,
                        "name": p.name,
                        "old_priority": p.old_priority,
                        "new_priority": p.new_priority,
                        "access_count": p.access_count,
                    })).collect::<Vec<_>>(),
                },
                "candidate_pool_health": build_pool_health_json(&pool_health),
                "stats": {
                    "total_units": all_units.len(),
                },
                "hint": "设置 dry_run=false 执行整合"
            }))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 数据获取
// ═══════════════════════════════════════════════════════════════════════════

/// 获取所有 CU（不过滤）
///
/// **双层保护机制**（由各分析函数内部执行）：
///
/// 1. **schema 元数据保护**：`is_a` 含 `kind` / `prop` / `meta` / `relation` / `cu`
///    的 CU 永不被遗忘、永不参与晋升（这些是认知 schema，不是知识）
/// 2. **候选池内保护**：`priority <= ENTER_PROMPT_THRESHOLD`（默认 20）的 CU
///    永不被遗忘（用户显式标记的重要记忆）
///
/// 不在此函数集中过滤——各分析函数按需过滤，避免漏判。
async fn fetch_all_non_schema_units(
    engine: &Arc<dyn AgentStore>,
) -> Result<Vec<CognitiveUnit>, String> {
    let page = engine
        .query(&FilterExpr::match_all(), &PageRequest::first(1000))
        .await
        .map_err(|e| format!("query 失败: {e}"))?;
    Ok(page.items)
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. 遗忘衰减分析
// ═══════════════════════════════════════════════════════════════════════════

struct ForgetReport {
    analyzed_count: usize,
    to_forget_ids: Vec<String>,
    details: Vec<ForgetDetail>,
}

struct ForgetDetail {
    id: String,
    retention: f64,
}

fn analyze_forget_decay(units: &[CognitiveUnit], now: u64, threshold: f32) -> ForgetReport {
    let mut details = Vec::new();
    let mut to_forget_ids = Vec::new();

    for cu in units {
        // **软删除信号**（最先检查，绕过双层保护）
        // confidence=0 表示 LLM 显式标记删除 → 立即遗忘
        // 这是 LLM 触发"删除"的标准方式（替代已废除的 memory.delete 操作）
        // 优先级最高：即使 priority<=20（候选池内）也强制遗忘
        if cu.confidence() <= 0.0 {
            to_forget_ids.push(cu.id().to_string());
            details.push(ForgetDetail {
                id: cu.id().to_string(),
                retention: 0.0,
            });
            continue;
        }

        // **双层保护**（软删除信号之后）：
        // 1. schema 元数据（is_a 含 kind/prop/meta/relation/cu）→ 永不被遗忘
        // 2. 候选池内（priority <= ENTER_PROMPT_THRESHOLD）→ 永不被遗忘
        if is_schema_metadata(cu) {
            continue;
        }
        if is_in_prompt_candidate(cu) {
            continue;
        }

        // Q2: confidence 入遗忘公式
        // retention = confidence × belief × exp(-Δt / memory_strength)
        // - confidence: LLM 写入时给定的可靠度（默认 0.7），低 confidence 加速遗忘
        // - belief: 动态信念度（被访问则升、被遗忘则降）
        // - memory_strength: 记忆强度（CU 自身的衰减常数）
        let confidence = cu.confidence() as f64;
        let belief = cu.meta_belief() as f64;
        let last = cu.last_access().unwrap_or_else(|| cu.created_at());
        let delta_secs = now.saturating_sub(last);
        let delta_days = delta_secs as f64 / 86400.0;
        let memory_strength = cu.memory_strength();

        // 艾宾浩斯遗忘曲线（含 confidence 修正）
        let retention = confidence * belief * (-delta_days / memory_strength).exp();

        if (retention as f32) < threshold {
            // 双层保护已过滤，此处的所有 CU 都允许遗忘
            to_forget_ids.push(cu.id().to_string());
            details.push(ForgetDetail {
                id: cu.id().to_string(),
                retention,
            });
        }
    }

    ForgetReport {
        analyzed_count: units.len(),
        to_forget_ids,
        details,
    }
}

/// 执行遗忘删除
async fn apply_forget(engine: &Arc<dyn AgentStore>, ids: &[String]) -> usize {
    let mut count = 0usize;
    for id in ids {
        if engine.delete(id).await.is_ok() {
            count += 1;
        }
    }
    count
}

// ═══════════════════════════════════════════════════════════════════════════
// 保护判定辅助函数
// ═══════════════════════════════════════════════════════════════════════════

/// 是否 schema 元数据（认知 schema，is_a 含 kind/prop/meta/relation/cu）
///
/// **设计意图**：元数据是认知体系的"语法"，不是"知识"——LLM 不会主动访问（因为
/// 提示词里已经知道），但它们是系统运行的基础。**绝不能被遗忘机制清理**。
fn is_schema_metadata(cu: &CognitiveUnit) -> bool {
    cu.get("is_a")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|v| {
                matches!(
                    v.as_str(),
                    Some("kind") | Some("prop") | Some("meta") | Some("relation") | Some("cu")
                )
            })
        })
        .unwrap_or(false)
}

/// 是否在系统提示词候选池内（`priority <= ENTER_PROMPT_THRESHOLD`）
///
/// **设计意图**：候选池内的 CU 是用户显式标记"重要"的记忆——即使 belief 衰减、
/// 长期不访问，也**绝不能被遗忘**。这是"显式保护"。
///
/// 与 `is_schema_metadata` 区别：
/// - `is_schema_metadata`：系统层 schema（与 LLM 行为无关，由系统强制保护）
/// - `is_in_prompt_candidate`：用户层"重要"标记（由 LLM 通过 `priority` 字段表达）
fn is_in_prompt_candidate(cu: &CognitiveUnit) -> bool {
    cu.get_number(cu_fields::PRIORITY)
        .map(|n| n as i64 <= ENTER_PROMPT_THRESHOLD)
        .unwrap_or(false) // 无 priority 字段 → 不在候选池（默认不保护）
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. 冗余合并分析
// ═══════════════════════════════════════════════════════════════════════════

struct MergeReport {
    analyzed_count: usize,
    merges: Vec<MergeCandidate>,
}

struct MergeCandidate {
    keep_id: String,
    keep_name: String,
    remove_id: String,
    remove_name: String,
    similarity: f32,
}

fn analyze_redundancy(units: &[CognitiveUnit], threshold: f32) -> MergeReport {
    // 只对有 embedding 的 CU 计算两两相似度（O(n²)，但 CU 数量通常 < 1000）
    let with_embedding: Vec<&CognitiveUnit> = units
        .iter()
        .filter(|cu| cu.get_embedding().is_some())
        .collect();

    let mut merges = Vec::new();
    let mut removed_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for i in 0..with_embedding.len() {
        if removed_ids.contains(with_embedding[i].id()) {
            continue;
        }
        for j in (i + 1)..with_embedding.len() {
            if removed_ids.contains(with_embedding[j].id()) {
                continue;
            }
            let a = with_embedding[i];
            let b = with_embedding[j];

            // 优先合并同类型的 CU
            let same_type = a.is_a().iter().any(|t| b.is_type(t));
            if !same_type {
                continue;
            }

            let emb_a = a.get_embedding().unwrap();
            let emb_b = b.get_embedding().unwrap();
            let sim = cosine_similarity(&emb_a, &emb_b);

            if sim >= threshold {
                // 保留 belief 更高者
                let (keep, remove) = if a.meta_belief() >= b.meta_belief() {
                    (a, b)
                } else {
                    (b, a)
                };
                merges.push(MergeCandidate {
                    keep_id: keep.id().to_string(),
                    keep_name: keep.name().unwrap_or("").to_string(),
                    remove_id: remove.id().to_string(),
                    remove_name: remove.name().unwrap_or("").to_string(),
                    similarity: sim,
                });
                removed_ids.insert(remove.id().to_string());
            }
        }
    }

    MergeReport {
        analyzed_count: with_embedding.len(),
        merges,
    }
}

/// 执行合并：保留高 belief CU，累加 access_count，删除被合并 CU
async fn apply_merge(engine: &Arc<dyn AgentStore>, merges: &[MergeCandidate]) -> usize {
    let mut count = 0usize;
    for m in merges {
        // 把被合并 CU 的 access_count 累加到保留者
        if let Ok(Some(mut keeper)) = engine.get(&m.keep_id).await {
            if let Ok(Some(removed)) = engine.get(&m.remove_id).await {
                let acc = keeper.access_count() + removed.access_count();
                keeper.set("_ext_access_count", json!(acc));
                // 合并也提升 belief（两个 CU 确认同一认知 → 信念更强）
                keeper.bump_meta_belief(0.02);
                let _ = engine.update(&keeper).await;
            }
        }
        // 删除被合并 CU
        if engine.delete(&m.remove_id).await.is_ok() {
            count += 1;
        }
    }
    count
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. 优先级晋升分析
// ═══════════════════════════════════════════════════════════════════════════

struct PromotionReport {
    analyzed_count: usize,
    to_promote: Vec<PromotionCandidate>,
}

struct PromotionCandidate {
    id: String,
    name: String,
    old_priority: usize,
    new_priority: usize,
    access_count: u64,
}

fn analyze_priority_promotion(units: &[CognitiveUnit]) -> PromotionReport {
    let mut candidates = Vec::new();

    for cu in units {
        // schema 元数据（is_a 含 kind/prop/meta/relation/cu）不参与晋升
        // 这些是认知 schema，不是知识
        if is_schema_metadata(cu) {
            continue;
        }
        let access = cu.access_count();
        if access < PROMOTION_ACCESS_COUNT {
            continue;
        }
        let priority = cu.get("priority").and_then(|v| v.as_u64()).unwrap_or(99) as usize;
        if priority > PROMOTION_PRIORITY_CEILING {
            let new_priority = priority.saturating_sub(PROMOTION_STEP);
            candidates.push(PromotionCandidate {
                id: cu.id().to_string(),
                name: cu.name().unwrap_or("").to_string(),
                old_priority: priority,
                new_priority,
                access_count: access,
            });
        }
    }

    PromotionReport {
        analyzed_count: units.len(),
        to_promote: candidates,
    }
}

/// 执行优先级晋升
async fn apply_promotion(engine: &Arc<dyn AgentStore>, candidates: &[PromotionCandidate]) -> usize {
    let mut count = 0usize;
    for p in candidates {
        if let Ok(Some(mut cu)) = engine.get(&p.id).await {
            cu.set("priority", json!(p.new_priority));
            if engine.update(&cu).await.is_ok() {
                count += 1;
            }
        }
    }
    count
}

crate::submit_cognition_op!(ConsolidateOp);

// ═══════════════════════════════════════════════════════════════════════════
// 4. 候选池健康报告（只报告不越权）
// ═══════════════════════════════════════════════════════════════════════════

/// 候选池健康报告的阈值：meta_belief 低于此值的 CU 被标记为"信念偏低"
const POOL_LOW_BELIEF_THRESHOLD: f64 = 0.3;
/// 候选池健康报告的阈值：超过此天数未访问的 CU 被标记为"长期未访问"
const POOL_STALE_DAYS: f64 = 30.0;

struct PoolHealthReport {
    /// 候选池中的 CU 总数
    pool_size: usize,
    /// 信念偏低的 CU
    low_belief: Vec<PoolHealthEntry>,
    /// 长期未访问的 CU
    stale_access: Vec<PoolHealthEntry>,
}

struct PoolHealthEntry {
    id: String,
    name: String,
    /// meta_belief 值
    belief: f64,
    /// 距上次访问的天数
    days_since_access: f64,
}

impl Clone for PoolHealthEntry {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            name: self.name.clone(),
            belief: self.belief,
            days_since_access: self.days_since_access,
        }
    }
}

/// 分析候选池健康度（只报告事实，不修改任何数据）
///
/// 扫描 `priority <= ENTER_PROMPT_THRESHOLD` 的 CU，报告两类值得关注的条目：
/// 1. **信念偏低**（`meta_belief < 0.3`）：信念度较低，LLM 可能需审视是否仍有价值
/// 2. **长期未访问**（距上次访问 > 30 天）：一直没被检索到，LLM 可能需审视
///
/// **设计原则**：系统绝不建议"该踢出谁"，只列出客观事实让 LLM 自主判断。
fn analyze_candidate_pool_health(units: &[CognitiveUnit], now: u64) -> PoolHealthReport {
    let mut low_belief = Vec::new();
    let mut stale_access = Vec::new();
    let mut pool_size = 0usize;

    for cu in units {
        // schema 元数据不纳入候选池健康统计
        if is_schema_metadata(cu) {
            continue;
        }
        // 只关注候选池内的 CU
        if !is_in_prompt_candidate(cu) {
            continue;
        }
        pool_size += 1;

        let belief = cu.meta_belief() as f64;
        let last = cu.last_access().unwrap_or_else(|| cu.created_at());
        let delta_secs = now.saturating_sub(last);
        let days_since_access = delta_secs as f64 / 86400.0;

        let entry = PoolHealthEntry {
            id: cu.id().to_string(),
            name: cu.name().unwrap_or("").to_string(),
            belief,
            days_since_access,
        };

        if belief < POOL_LOW_BELIEF_THRESHOLD {
            low_belief.push(entry.clone());
        }
        if days_since_access > POOL_STALE_DAYS {
            stale_access.push(entry);
        }
    }

    PoolHealthReport {
        pool_size,
        low_belief,
        stale_access,
    }
}

/// 将候选池健康报告序列化为 JSON Value（用于 OperationResult）
fn build_pool_health_json(report: &PoolHealthReport) -> Value {
    json!({
        "pool_size": report.pool_size,
        "low_belief_count": report.low_belief.len(),
        "low_belief": report.low_belief.iter().map(|e| json!({
            "id": e.id,
            "name": e.name,
            "meta_belief": e.belief,
        })).collect::<Vec<_>>(),
        "stale_access_count": report.stale_access.len(),
        "stale_access": report.stale_access.iter().map(|e| json!({
            "id": e.id,
            "name": e.name,
            "days_since_access": (e.days_since_access.round() as u64),
        })).collect::<Vec<_>>(),
        "note": "此报告仅列事实，不自动修改任何 CU。LLM 可根据此报告自主调用 memory.save 调整。"
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests;
