//! 系统提示词预算分配器
//!
//! 本模块集中实现"动态构建系统提示词"所需的所有底层机制：
//!
//! - [`estimate_tokens`]：CJK 感知的 token 估算（避免简单 chars/4 在中文场景严重失真）
//! - [`PromptBudget`]：预算配置（总预算 + 固定开销预算）
//! - [`BudgetUsage`]：实际用量追踪（按 section 分解）
//! - [`CuScore`]：单个 CU 的相关性评分（用于动态排序）
//! - [`compute_cu_score`]：综合 priority + meta_belief + 可选 relevance 计算 score
//!
//! ## 三层目标映射
//!
//! - **第 1 层 动态构建**：`estimate_tokens` + `CuScore` + 预算分配
//! - **第 2 层 LLM 自我管理**：[`PromptBudget`] 暴露给 LLM 内部使用（通过系统提示词末尾的"预算告警"段驱动）
//! - **第 3 层 限制与最大化**：[`BudgetUsage`] 用于在系统提示词末尾展示"预算状态"段

use serde::{Deserialize, Serialize};

/// 估算文本的 token 数
///
/// **精度策略**（在无 tokenizer 依赖场景下的最优近似）：
/// - CJK 字符（Unicode 范围 0x4E00-0x9FFF + 0x3400-0x4DBF + 全角符号）：
///   每个字符约 1 token（BPE 对 CJK 通常每个字拆为 1 个 token）
/// - 其它字符（拉丁字母、数字、空格、ASCII 符号）：
///   每 4 字符约 1 token（BPE 对 ASCII 文本通常按 4 字节聚合）
///
/// **与"chars/4"旧公式的对比**：
/// - 旧公式对 100 字中文 → 25 token（实际 ~100），误差 4 倍
/// - 新公式对 100 字中文 → 100 token（实际 ~100），误差 < 1 倍
/// - 对 400 字英文 → 100 token（实际 ~100），一致
///
/// **为什么不依赖 tiktoken**：
/// 1. 引入新依赖会改变构建矩阵（v3.0 之前的 MSRV 兼容性）
/// 2. 估算精度足够用于"预算管理"（不需要精确到 ±1 token）
/// 3. `count_tokens` 类的精确调用留给上层 LLM 路由（`MODEL_CHAT` 路径）
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk_count: usize = 0;
    let mut other_count: usize = 0;
    for c in text.chars() {
        if is_cjk(c) {
            cjk_count += 1;
        } else {
            other_count += 1;
        }
    }
    cjk_count + other_count.div_ceil(4)
}

/// 是否为 CJK 字符（粗略覆盖常用范围）
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'    // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}'  // CJK Extension A
        | '\u{F900}'..='\u{FAFF}'  // CJK Compatibility Ideographs
        | '\u{20000}'..='\u{2A6DF}' // CJK Extension B
        | '\u{2A700}'..='\u{2EBEF}' // CJK Extension C/D/E/F
        | '\u{3000}'..='\u{303F}'  // CJK Symbols and Punctuation
        | '\u{FF00}'..='\u{FFEF}'  // Halfwidth and Fullwidth Forms
    )
}

/// 系统提示词预算配置
///
/// **设计原则**：
/// - **总预算** = 模型上下文窗口中"留给系统提示词"的那部分
///   - 例如 8K 上下文模型，预留 4K 给系统提示词（剩下给对话历史 + 工具调用）
/// - **固定开销预算** = 不由 CU 数量决定的部分（identity 锚定、时间戳、工具速查、预算状态段等）
///   - 这部分先从总预算中扣除
/// - **CU 预算** = 总预算 − 固定开销（这是真正能塞多少条 CU 的预算）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptBudget {
    /// 系统提示词总预算（tokens）
    pub total: usize,
    /// 固定开销预算（tokens）—— 不由 CU 数量决定的部分
    pub overhead: usize,
}

impl PromptBudget {
    /// 构造 PromptBudget
    ///
    /// `overhead` 若 ≥ `total`，自动收缩到 `total.saturating_sub(1)`，避免 CU 预算为负
    pub fn new(total: usize, overhead: usize) -> Self {
        let overhead = overhead.min(total.saturating_sub(1));
        Self { total, overhead }
    }

    /// 留给 CU 的预算（实际能塞多少条 CU）
    pub fn available_for_cus(&self) -> usize {
        self.total.saturating_sub(self.overhead)
    }
}

impl Default for PromptBudget {
    /// 默认：3500 tokens 总预算，500 tokens 固定开销
    /// → 3000 tokens 给 CU
    fn default() -> Self {
        Self::new(3500, 500)
    }
}

/// 系统提示词预算实际使用情况
///
/// **用途**（v54 起，预算状态段正式落地）：
/// 1. 在系统提示词末尾渲染"预算状态段"——**纯事实**（用量 / 池子规模 / 截断数），
///    不含任何"建议踢出哪些 CU"的判断。价值判断完全交给 LLM 自主进化。
/// 2. 当超出预算时状态段升级为"告警"措辞，但仍只报事实。
/// 3. 提供给内部测试与诊断使用。
///
/// **设计意图**：状态段的目标是让 LLM **感知到容量压力**，由 LLM 审视自己的认知
/// 后自主决定踢出谁（调 `priority>20`，CU 仍保留）。系统绝不预计算"该删哪些"。
/// 删除必须由 LLM 显式 `memory.save {id, confidence: 0}` 触发。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetUsage {
    /// 已用 tokens（含 overhead 和 CU + 状态段自身）
    pub used: usize,
    /// 固定开销部分（overhead）实际占用
    pub overhead_used: usize,
    /// CU 部分实际占用
    pub cu_used: usize,
    /// 总预算
    pub total: usize,
    /// 各 section 的细分（section_name -> tokens）
    pub sections: Vec<(String, usize)>,
    /// 候选池统计（v54）：参与评分排序、争抢预算的 CU 总数
    pub candidate_total: usize,
    /// 候选池统计（v54）：实际写入提示词的 CU 数（其余被预算截断）
    pub candidate_shown: usize,
}

impl BudgetUsage {
    pub fn new(total: usize) -> Self {
        Self {
            used: 0,
            overhead_used: 0,
            cu_used: 0,
            total,
            sections: Vec::new(),
            candidate_total: 0,
            candidate_shown: 0,
        }
    }

    /// 添加一个 section 用量
    pub fn add_section(&mut self, name: impl Into<String>, tokens: usize) {
        self.sections.push((name.into(), tokens));
        self.used += tokens;
    }

    /// 剩余 tokens
    #[allow(dead_code)] // 保留供外部诊断使用
    pub fn remaining(&self) -> usize {
        self.total.saturating_sub(self.used)
    }

    /// 使用率（0.0 ~ 1.0+，超预算时 > 1.0）
    pub fn usage_ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.used as f64 / self.total as f64
        }
    }

    /// 因预算不足被截断的候选数（candidate_total − candidate_shown）
    pub fn candidate_truncated(&self) -> usize {
        self.candidate_total.saturating_sub(self.candidate_shown)
    }
}

/// 价值密度评分的 token 归一化基准（token 数）。
///
/// 一条 `tokens = TOKEN_NORM_UNIT` 的 CU，价值密度 = 原始价值；
/// tokens 翻倍则价值密度减半。这让"短而高价值"的 CU 在预算受限时
/// 优先保留，避免大块低价值 CU 挤占稀缺的系统提示词空间。
///
/// 基准取 50：典型单条 CU 渲染后约 30~80 token，50 是居中锚点。
pub const TOKEN_NORM_UNIT: usize = 50;

/// 价值分三维度权重（集中便于调参与测试）
const PRIORITY_WEIGHT: f64 = 0.4;
const BELIEF_WEIGHT: f64 = 0.3;
const RELEVANCE_WEIGHT: f64 = 0.3;

/// 单个 CU 的综合评分（价值密度模型，v2）
///
/// **评分公式**：
/// ```text
/// value = PRIORITY_WEIGHT × 1/(priority + 1)
///       + BELIEF_WEIGHT × meta_belief
///       + RELEVANCE_WEIGHT × relevance
///
/// score = value / max(1, tokens / TOKEN_NORM_UNIT)
/// ```
///
/// - `value`：原始价值分（[0, 1]），三维度加权——priority 小 / belief 高 / 相关性高 → 高价值
/// - `score`：价值**密度**（每 token 的价值），token 越多密度越低
///
/// **为何用密度而非纯价值**：系统提示词预算是稀缺资源。同样价值下，
/// 一条 50 token 的 CU 比一条 250 token 的 CU 更值得占用空间
/// （前者密度是后者 5 倍）。LLM 审视自己认知后仍可自主踢出任意 CU，
/// 系统只负责"用同样预算塞下更多高价值认知"。
///
/// 评分高的 CU 在预算受限时优先保留。
///
/// 字段可能未被直接读取（外部按需使用），故抑制 dead_code 警告。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CuScore {
    pub cu_id: String,
    /// 原始价值分（不含 token 密度归一化），范围 [0, 1]
    pub value: f64,
    /// 综合排序分 = value / max(1, tokens / TOKEN_NORM_UNIT)
    pub score: f64,
    pub priority: i64,
    pub meta_belief: f64,
    pub relevance: f64,
    pub estimated_tokens: usize,
}

/// 计算单个 CU 的综合评分（价值密度模型，v2）
///
/// **参数**：
/// - `cu_id`：CU 标识（仅用于 CuScore 返回值，标识用）
/// - `priority`：CU 的 priority 字段（值越小越重要；缺省 10）
/// - `meta_belief`：CU 的 meta_belief 字段（0.0~1.0，缺省 0.5）
/// - `relevance`：CU 与当前 query 的相关性（0.0~1.0，无 query 时传 0.0）
/// - `estimated_tokens`：CU 渲染后预计占用的 tokens
pub fn compute_cu_score(
    cu_id: impl Into<String>,
    priority: i64,
    meta_belief: f64,
    relevance: f64,
    estimated_tokens: usize,
) -> CuScore {
    // priority 越小越重要 → 用 1/(priority+1) 把它反转成"越大越好"
    let priority_part = 1.0 / (priority.max(0) as f64 + 1.0);
    // meta_belief 和 relevance 已在 0~1 范围，做防御性 clamp
    let value = PRIORITY_WEIGHT * priority_part
        + BELIEF_WEIGHT * meta_belief.clamp(0.0, 1.0)
        + RELEVANCE_WEIGHT * relevance.clamp(0.0, 1.0);
    // 价值密度：token 越多，单位 token 价值越低
    // max(1, ...) 保证至少除以 1（避免除零），且让 0 token 的 CU 不被无限放大
    let token_factor = ((estimated_tokens / TOKEN_NORM_UNIT) as f64).max(1.0);
    let score = value / token_factor;
    CuScore {
        cu_id: cu_id.into(),
        value,
        score,
        priority,
        meta_belief,
        relevance,
        estimated_tokens,
    }
}

#[cfg(test)]
#[path = "prompt_budget_tests.rs"]
mod tests;
