use crate::plugins::agent::core::types::cu_fields;
use crate::plugins::agent::core::{
    compute_cu_score, estimate_tokens, AgentStore, BudgetUsage, CognitiveUnit, CuScore,
    PromptBudget,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use time::{format_description, OffsetDateTime};

/// `build_persona()` 返回值：组装好的人格文本 + 实际预算使用情况
///
/// 拆分返回而非只返回 String，是为了让调用方能拿到 usage 做诊断
/// （预算状态段已内嵌 prompt 文本）。
#[derive(Debug, Clone)]
pub struct BuildResult {
    /// 完整的人格文本
    pub prompt: String,
    /// 实际预算使用情况（用于末尾渲染预算状态段）
    #[allow(dead_code)] // 调用方不读取 usage（状态已内嵌 prompt），保留供诊断/测试
    pub usage: BudgetUsage,
}

/// 构建人格文本（供 `agent_identity` 工具说明嵌入）
///
/// **重构说明**：人格不再写入 `system_prompt`，而是嵌入 `agent_identity`
/// 能力的 `description` 随工具定义送达 LLM（每轮请求自动可见）。
///
/// **三层目标映射**：
/// - **第 1 层 动态构建**：
///   - 使用 [`estimate_tokens`] 精确估算（不是 chars/4）
///   - 评分排序：`priority + meta_belief + relevance(query)` 综合打分
///   - 预算分配：按类型分配 token 上限
/// - **第 2 层 LLM 自我管理**：
///   - 返回 [`BuildResult`]（含 [`BudgetUsage`]），让上层可暴露给 LLM
///   - 在末尾追加"预算状态段" + 工具速查，引导 LLM 主动管理
/// - **第 3 层 限制与最大化**：
///   - 末尾追加"提示词预算"段，让 LLM 知道总预算 / 已用 / 剩余
///   - LLM 知道有限制 → 主动通过 `memory.manage_prompt` 整理以腾出空间
///
/// **结构**：
/// ```text
/// 当前时间：...
///
/// ## 身份
/// - **id**: `identity`
/// ...
///
/// ## 行为规则（3 项）
/// - **id**: `xxx`
///   **description**: ...
///
/// ## 工具速查
/// - memory.view_prompt — 查阅预算
/// - memory.manage_prompt — 整理
/// - agent_cognition (memory.save) — 保存新认知
///
/// ## 提示词预算
/// - 总预算：3500 tokens
/// - 已用：~1247 tokens (36%)
/// - 剩余：~2253 tokens
/// - 主动整理 = 更多重要知识空间
/// ```
///
/// **参数**：
/// - `store`：AgentStore（用于查询 sys-level CU）
/// - `budget`：提示词预算（总预算 + 固定开销）
/// - `relevance_query`：可选，用于相关性排序的查询文本（用户消息）
///
/// **变更历史**：
/// - v1：硬编码 budget = 3500, max_per_type = 10, char/4 估算
/// - v2 (I-065)：重构为 PromptBudget + estimate_tokens + 多维评分 + 预算状态段
/// - v3（agent 降级为普通插件）：系统提示词形态（`build` + `RenderStyle`）移除，
///   人格文本成为唯一输出形态
pub async fn build_persona(
    store: &dyn AgentStore,
    budget: &PromptBudget,
    relevance_query: Option<&str>,
) -> BuildResult {
    let mut usage = BudgetUsage::new(budget.total);

    let all_sys_units = get_all_system_units(store).await;

    // ── 身份锚定（关键修复）──
    // 身份 CU 是系统提示词中最重要的锚点，必须始终出现。
    // 但各 agent 的 `identity.yaml` 通常**不带 `priority` 字段**，
    // 而 `get_all_system_units` 用 `priority <= 20` 过滤会把它们整体丢掉
    // （`evaluate_filter` 对缺失字段返回 None → 不满足 `<=` → 被排除）。
    // 结果：系统提示词里完全没有 `## 身份` 段，模型失去身份约束，
    // 凭空臆造（最常见是误用 agent 目录里排在第一位的 normal「系统管家」身份）。
    // 这正表现为「子智能体调用时 agent_id 没生效 / 路由到了默认 agent」。
    // 修复：优先从过滤后的集合取 identity；取不到就**直连 store** 拿 identity，
    // 彻底绕过 priority 过滤，保证身份永远进入系统提示词。
    let identity_cu = match all_sys_units.get("identity") {
        Some(cu) => Some(cu.clone()),
        None => store.get("identity").await.ok().flatten(),
    };

    // ── 1. 渲染 overhead 段（身份锚定 + 时间戳） ──
    // 人格形态：直接以时间戳开头（无"## 系统提示"总标题），
    // 便于嵌入 agent_identity 工具说明。
    let mut prompt = String::new();
    let header = format!("当前时间：{}\n\n", format_time());
    usage.add_section("overhead", estimate_tokens(&header));
    prompt.push_str(&header);

    if let Some(ref cu) = identity_cu {
        let section_title = "## 身份\n\n";
        let body = render_cu_markdown(cu);
        let section = format!("{}{}", section_title, body);
        // usage 标记为 identity 段（overhead 部分）
        usage.overhead_used += estimate_tokens(&section);
        usage.add_section("identity", 0); // 占位以保持顺序（实际计入 overhead）
                                          // 修正：把上面 add_section 的 0 替换为真实值
        if let Some(last) = usage.sections.last_mut() {
            if last.0 == "identity" {
                last.1 = estimate_tokens(&section);
            }
        }
        usage.used += estimate_tokens(&section);
        prompt.push_str(&section);
        prompt.push('\n');
    }

    // ── 2. 按 is_a[0] 分组（排除 identity） ──
    let mut by_type: HashMap<String, Vec<&CognitiveUnit>> = HashMap::new();
    for u in all_sys_units.values() {
        if u.id() == "identity" {
            continue;
        }
        if let Some(first_type) = u.is_a_list().first() {
            by_type
                .entry(first_type.id.to_string())
                .or_default()
                .push(u);
        }
    }

    if !by_type.is_empty() {
        // ── 3. 按元认知优先级排序类型 ──
        let type_names: Vec<String> = by_type.keys().cloned().collect();
        let mut type_priorities: Vec<(String, i64)> = Vec::with_capacity(type_names.len());
        for t in &type_names {
            let priority = get_metacognitive_priority(store, t.as_str()).await;
            type_priorities.push((t.clone(), priority));
        }
        type_priorities.sort_by_key(|&(_, p)| p);
        by_type.retain(|_, v| !v.is_empty());

        // ── 4. 计算每个 CU 的评分（多维：priority + meta_belief + relevance） ──
        let query_tokens = relevance_query
            .map(tokenize_for_relevance)
            .unwrap_or_default();

        // 对每个 type 内的 CU 计算评分并降序
        let mut scored_by_type: HashMap<String, Vec<(&CognitiveUnit, CuScore)>> = HashMap::new();
        for (au_type, units) in &by_type {
            let mut scored: Vec<(&CognitiveUnit, CuScore)> = Vec::with_capacity(units.len());
            for cu in units {
                let priority = cu
                    .get_number(cu_fields::PRIORITY)
                    .map(|n| n as i64)
                    .unwrap_or(99);
                let meta_belief = cu.meta_belief() as f64;
                let relevance = compute_relevance(cu, &query_tokens);
                let tokens = estimate_tokens(&render_cu_markdown(cu));
                let score = compute_cu_score(cu.id(), priority, meta_belief, relevance, tokens);
                scored.push((cu, score));
            }
            // 评分降序
            scored.sort_by(|a, b| {
                b.1.score
                    .partial_cmp(&a.1.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            scored_by_type.insert(au_type.clone(), scored);
        }

        // ── 5. 按预算分配到各 type ──
        let cu_budget = budget.available_for_cus();
        let type_budgets = allocate_type_budget(&type_priorities, &scored_by_type, cu_budget);

        // ── 6. 渲染各 type 的 CU ──
        render_cognitive_units(
            &mut prompt,
            &scored_by_type,
            &type_priorities,
            &type_budgets,
            &mut usage,
        );
    }

    // ── 7. 追加预算状态段（纯事实，零建议）──
    let status_section = render_budget_status(&usage);
    let status_tokens = estimate_tokens(&status_section);
    // 预算状态段自身也算在 used 里（让 LLM 看到的是包含状态段的完整用量）
    usage.add_section("budget_status", status_tokens);
    prompt.push_str("\n\n");
    prompt.push_str(&status_section);

    BuildResult { prompt, usage }
}

/// 预算使用率告警阈值（0.0 ~ 1.0）。
///
/// 超过此比例时，状态段标题从"预算状态"升级为"⚠️ 预算告警"，
/// 提醒 LLM 容量压力大，可能需要主动整理认知。
const BUDGET_WARN_THRESHOLD: f64 = 0.85;

/// 渲染预算状态段（纯事实，不含任何"建议踢出哪些 CU"的判断）
///
/// **设计原则**：系统负责"压力感知"，LLM 负责"价值判断"。
/// 状态段只报告用量/池子规模/截断数等客观事实，不预计算"该删哪些"。
/// LLM 看到事实后自主决定是否调用 memory.save / memory.consolidate。
///
/// **输出格式**：
/// ```text
/// ## ⚠️ 预算告警        ← 超阈值时带 ⚠️
/// - 已用：1247 / 3500 tokens（36%）
/// - 候选池：共 45 项，展示 32 项，13 项因预算未展示
/// - 管理工具：memory.save（priority>20 移出候选池 / confidence:0 物理删除）| memory.consolidate（批量整理）
/// ```
fn render_budget_status(usage: &BudgetUsage) -> String {
    let ratio = usage.usage_ratio();
    let is_warning = ratio >= BUDGET_WARN_THRESHOLD;

    let title = if is_warning {
        "## ⚠️ 预算告警"
    } else {
        "## 预算状态"
    };

    let pct = (ratio * 100.0).round() as usize;
    let header = format!(
        "- 已用：{} / {} tokens（{}%）",
        usage.used, usage.total, pct
    );

    let truncated = usage.candidate_truncated();
    let pool_line = format!(
        "- 候选池：共 {} 项，展示 {} 项，{} 项因预算未展示",
        usage.candidate_total, usage.candidate_shown, truncated
    );

    // 工具速查——只列能力，不暗示哪些该用
    let tools_line = "- 管理工具：memory.save（priority>20 移出候选池 / confidence:0 物理删除）| memory.consolidate（批量整理）";

    let mut out = String::new();
    out.push_str(title);
    out.push('\n');
    out.push_str(&header);
    out.push('\n');
    out.push_str(&pool_line);
    out.push('\n');
    out.push_str(tools_line);
    out
}

/// 分配各 type 的 token 预算
///
/// **策略**：
/// - 优先按 CU 数量比例分配（评分高的 type 获得更多预算）
/// - 任何 type 至少分配 50 tokens（保证至少 1 条 CU 能展示）
/// - 总和不超过 `cu_budget`
fn allocate_type_budget(
    type_priorities: &[(String, i64)],
    scored_by_type: &HashMap<String, Vec<(&CognitiveUnit, CuScore)>>,
    cu_budget: usize,
) -> HashMap<String, usize> {
    if cu_budget == 0 || type_priorities.is_empty() {
        return HashMap::new();
    }

    // 统计各 type 的"理想展示 tokens"（按评分取所有 CU）
    let mut ideal_tokens: HashMap<String, usize> = HashMap::new();
    for (au_type, _) in type_priorities {
        let total: usize = scored_by_type
            .get(au_type)
            .map(|v| v.iter().map(|(_, s)| s.estimated_tokens).sum())
            .unwrap_or(0);
        ideal_tokens.insert(au_type.clone(), total);
    }

    let total_ideal: usize = ideal_tokens.values().sum();
    let mut budgets: HashMap<String, usize> = HashMap::new();
    if total_ideal == 0 {
        return budgets;
    }

    if total_ideal <= cu_budget {
        // 预算充足：每个 type 拿到它的 ideal tokens
        return ideal_tokens;
    }

    // 预算不足：按比例缩放
    for (au_type, ideal) in &ideal_tokens {
        let share = (*ideal as f64 / total_ideal as f64 * cu_budget as f64) as usize;
        // 至少 50 tokens（保证至少 1 条 CU 能展示）
        budgets.insert(au_type.clone(), share.max(50));
    }

    // 调整：若 sum 超过 cu_budget，按优先级从低到高截断
    let sum: usize = budgets.values().sum();
    if sum > cu_budget {
        // 先把 (type, budget, priority) 收集出来再做调整——避免 &budgets 不变借用冲突
        let mut type_info: Vec<(String, usize, i64)> = budgets
            .iter()
            .map(|(t, b)| {
                let p = type_priorities
                    .iter()
                    .find(|(tp, _)| tp == t)
                    .map(|(_, p)| *p)
                    .unwrap_or(99);
                (t.clone(), *b, p)
            })
            .collect();
        // 按 priority 升序排序（小优先 → 先保留）
        type_info.sort_by_key(|&(_, _, p)| p);
        let mut remaining = cu_budget;
        let mut new_budgets: HashMap<String, usize> = HashMap::new();
        for (t, b, _) in type_info {
            if remaining == 0 {
                new_budgets.insert(t, 0);
            } else if b > remaining {
                new_budgets.insert(t, remaining);
                remaining = 0;
            } else {
                new_budgets.insert(t, b);
                remaining -= b;
            }
        }
        return new_budgets;
    }

    budgets
}

/// 把字符串切成 token（用于相关性匹配）
///
/// **简化策略**：
/// - CJK 字符：每个字单独成一个 token
/// - 其它字符：按空白切分
/// - 全部转小写
fn tokenize_for_relevance(text: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let mut current = String::new();
    for c in text.chars() {
        if is_cjk_char(c) {
            // CJK：先 flush 当前 ASCII token，再单字成 token
            if !current.is_empty() {
                tokens.insert(current.to_lowercase());
                current.clear();
            }
            tokens.insert(c.to_string());
        } else if c.is_alphanumeric() {
            current.push(c);
        } else if !current.is_empty() {
            tokens.insert(current.to_lowercase());
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.insert(current.to_lowercase());
    }
    tokens
}

fn is_cjk_char(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FFEF}'
    )
}

/// 计算 CU 与查询的相关性（Jaccard 相似度，0.0~1.0）
///
/// CU 的匹配文本 = name + description + content
fn compute_relevance(cu: &CognitiveUnit, query_tokens: &HashSet<String>) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let cu_text = format!(
        "{} {} {}",
        cu.name().unwrap_or(""),
        cu.description().unwrap_or(""),
        cu.content().unwrap_or(""),
    );
    let cu_tokens = tokenize_for_relevance(&cu_text);
    if cu_tokens.is_empty() {
        return 0.0;
    }
    let intersection = query_tokens.intersection(&cu_tokens).count();
    let union = query_tokens.union(&cu_tokens).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

// ── 渲染辅助函数 ──

fn format_time() -> String {
    let now = OffsetDateTime::now_utc();
    // time 0.3.55 起 parse 被 deprecated，parse_borrowed::<2> 为等价替代
    let fmt = format_description::parse_borrowed::<2>(
        "[year]年[month]月[day]日 [hour]:[minute]:[second] UTC",
    )
    .expect("hardcoded time format must parse");
    now.format(&fmt).unwrap_or_else(|_| "未知时间".to_string())
}

/// 把单个 CU 渲染为 markdown bullet list（内容 = to_llm_value 的全部字段）
fn render_cu_markdown(cu: &CognitiveUnit) -> String {
    use std::collections::BTreeMap;

    let value = cu.to_llm_value();
    let obj = match value.as_object() {
        Some(obj) => obj,
        None => return format!("- id: {}\n", cu.id()),
    };

    let preferred = [
        cu_fields::ID,
        cu_fields::NAME,
        cu_fields::DESCRIPTION,
        cu_fields::IS_A,
        cu_fields::PRIORITY,
        cu_fields::CONFIDENCE,
    ];

    let mut out = String::new();
    let mut emitted: HashSet<String> = HashSet::new();

    for key in preferred {
        if let Some(v) = obj.get(key) {
            if key == cu_fields::ID {
                out.push_str(&format!("- **{}**: `{}`\n", key, value_to_inline(v)));
            } else {
                out.push_str(&format!("- **{}**: {}\n", key, value_to_inline(v)));
            }
            emitted.insert(key.to_string());
        }
    }
    let rest: BTreeMap<&String, &Value> =
        obj.iter().filter(|(k, _)| !emitted.contains(*k)).collect();
    for (k, v) in &rest {
        out.push_str(&format!("- **{}**: {}\n", k, value_to_inline(v)));
    }
    out
}

fn value_to_inline(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// 渲染系统级 CU（按 type 优先级 + 评分排序 + 预算截断）
///
/// 截断统计：每组的 `total`（候选数）与 `shown`（展示数）累加到
/// `usage.candidate_total / candidate_shown`，供预算状态段报告"截断 N 项"。
/// **不记录被截断的具体 id**——价值判断交给 LLM，避免系统暗示"这些可弃"。
fn render_cognitive_units(
    prompt: &mut String,
    scored_by_type: &HashMap<String, Vec<(&CognitiveUnit, CuScore)>>,
    type_priorities: &[(String, i64)],
    type_budgets: &HashMap<String, usize>,
    usage: &mut BudgetUsage,
) {
    for (au_type, _) in type_priorities {
        let items = match scored_by_type.get(au_type) {
            Some(items) => items,
            None => continue,
        };
        let budget = type_budgets.get(au_type).copied().unwrap_or(0);
        if budget == 0 || items.is_empty() {
            continue;
        }

        let total = items.len();
        let scenario = business_scenario_label(au_type);
        let section_title = format!("## {}（共 {} 项）\n\n", scenario, total);
        prompt.push_str(&section_title);

        let mut section_tokens = estimate_tokens(&section_title);
        let mut shown = 0;
        for (cu, score) in items.iter() {
            if section_tokens + score.estimated_tokens > budget {
                break;
            }
            let body = render_cu_markdown(cu);
            prompt.push_str(&body);
            prompt.push('\n');
            section_tokens += score.estimated_tokens + estimate_tokens(&body); // 防止 markdown 渲染与 toString 微小差异
            shown += 1;
        }

        // 候选池统计（v54）：累加供预算状态段报告截断规模
        usage.candidate_total += total;
        usage.candidate_shown += shown;

        if total > shown {
            let tail = format!(
                "> 本组还有 {} 项因预算未展示（可用 memory.retrieve 按需查询）。\n\n",
                total - shown
            );
            prompt.push_str(&tail);
            section_tokens += estimate_tokens(&tail);
        }

        usage.add_section(scenario, section_tokens);
        usage.cu_used += section_tokens;
    }
}

/// `is_a[0]` → 业务场景标题
fn business_scenario_label(au_type: &str) -> String {
    match au_type {
        "rule" => "行为规则".to_string(),
        "skill" => "专业技能".to_string(),
        "strategy" => "思维策略".to_string(),
        "judgment" => "判断准则".to_string(),
        "experience" => "经验教训".to_string(),
        "fact" => "事实知识".to_string(),
        other => other.to_string(),
    }
}

/// 从 AgentStore 查询元认知单元的优先级
async fn get_metacognitive_priority(store: &dyn AgentStore, type_id: &str) -> i64 {
    use crate::plugins::agent::core::FilterExpr;
    use crate::plugins::agent::core::PageRequest;
    use serde_json::json;

    let filter = FilterExpr::eq(cu_fields::ID, json!(type_id));
    match store.query(&filter, &PageRequest::first(1)).await {
        Ok(result) => result
            .items
            .first()
            .and_then(|cu| cu.get_number(cu_fields::PRIORITY))
            .map(|n| n as i64)
            .unwrap_or(99),
        Err(e) => {
            crate::plugin_warn!(
                "agent",
                "[SystemPrompt] Failed to query metacognitive unit {}: {}",
                type_id,
                e
            );
            99
        }
    }
}

// ── 数据获取辅助函数 ──

/// 进入系统提示词的判定规则
///
/// `priority <= ENTER_PROMPT_THRESHOLD` 的 CU 都会进入系统提示词候选池，
/// 然后按 budget 截断、按 is_a 分组、同组内按 score 排序。
///
/// `priority` 是 LLM 唯一需要关注的字段（**只在同 kind/is_a 内比较**）：
/// - `0`     → 强制排在最前
/// - `1-20`  → 候选池内（值小排前）
/// - `> 20`  → 不进入系统提示词（按需检索）
///
/// 默认 `priority = 10`：新写入的 CU 自动进入候选池的中间位置。
///
/// **为什么阈值 = 20？**
/// - 系统提示词按 `is_a`（kind）分组展示（identity / rule / judgment / strategy / tone...）
/// - 阈值大小只需支持"同 kind 内"的精细排序
/// - 10 级足够区分"很前 / 居中 / 靠后"，20 级给双层余量
pub const ENTER_PROMPT_THRESHOLD: i64 = 20;

async fn get_all_system_units(store: &dyn AgentStore) -> HashMap<String, CognitiveUnit> {
    use crate::plugins::agent::core::FilterExpr;
    use crate::plugins::agent::core::PageRequest;

    let filter = FilterExpr::Lte {
        key: cu_fields::PRIORITY.to_string(),
        value: ENTER_PROMPT_THRESHOLD as f64,
    };
    let page = PageRequest::first(500);
    match store.query(&filter, &page).await {
        Ok(result) => result
            .items
            .into_iter()
            .filter_map(|u| {
                let id = u.id().to_string();
                if id.is_empty() {
                    return None;
                }
                Some((id, u))
            })
            .collect(),
        Err(e) => {
            crate::plugin_warn!(
                "agent",
                "[SystemPrompt] Failed to query priority-filtered units: {}",
                e
            );
            HashMap::new()
        }
    }
}

#[cfg(test)]
#[path = "system_prompt_tests.rs"]
mod tests;
