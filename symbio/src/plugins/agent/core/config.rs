use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackendType {
    Dir,
    Sqlite,
    Mindscape,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageFormat {
    Json,
    Yaml,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_storage_backend")]
    pub storage_backend: StorageBackendType,
    /// 基础存储后端类型（供 EmbeddingStore/MindscapeScaffold 内部使用）
    /// 默认为 Dir，仅在 storage_backend 为 Embedding/Mindscape 时生效
    #[serde(default = "default_base_backend")]
    pub base_backend: StorageBackendType,
    #[serde(default = "default_storage_format")]
    pub storage_format: StorageFormat,
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_categories")]
    pub categories: Vec<String>,
    #[serde(flatten)]
    pub backend_config: serde_json::Value,
    /// 认知反馈阈值（/ S-3）
    ///
    /// 这些阈值原本是 `cognitive_feedback.rs` 里的 `const`，
    /// 无法在 agents.yaml 中调整。v28 改为可配置。
    /// - `belief_flush_threshold`：`belief_buffer` 攒到多少 unit 时自动 flush
    /// - `conflict_cache_capacity`：`conflict_cache`（去重 HashSet）容量上限
    /// - `belief_boost_per_use`：每次访问的 meta_belief 增量
    /// - `belief_ceiling`：meta_belief 上限（防止 clamp 到 1.0）
    ///
    /// **兼容性**：老配置没有这些字段，`#[serde(default)]` 兜底为 v28 默认值。
    /// 如出现 `deserialize` 错误，请检查 YAML 是否被外部工具锁版本。
    #[serde(default)]
    pub cognition: CognitionThresholds,
    /// 系统提示词 token 预算（默认 3500）
    ///
    /// **三层目标映射（第 3 层 限制与最大化）**：
    /// - 控制 system message 的总长度，避免挤占对话历史 + 工具调用的上下文
    /// - 在 system_prompt 末尾追加"预算状态"段，让 LLM 知道剩余预算
    /// - 当超出预算时主动追加"预算告警"段，提示 LLM 优化提示词条目
    ///
    /// **预算告警 ≠ 删除记忆**：告警的目标是**优化系统提示词占用**——
    /// 提示 LLM 调整 `priority` 把低价值的 CU 踢出系统提示词候选池
    /// （设 `priority > 20`，CU 仍完整保留在库中，需要时通过 `memory.retrieve` 调出）。
    /// 真正的删除必须由 LLM 显式 `memory.save {id, confidence: 0}` 触发。
    #[serde(default = "default_prompt_budget_tokens")]
    pub prompt_budget_tokens: usize,
    /// 系统提示词固定开销预算（默认 500 tokens）
    ///
    /// 涵盖：identity 锚定、时间戳、工具速查、预算状态段等不由 CU 数量决定的部分
    /// 从总预算中扣除后，剩余部分才用于展示 CU
    #[serde(default = "default_prompt_overhead_tokens")]
    pub prompt_overhead_tokens: usize,
    /// 系统提示词预算使用率告警阈值（默认 0.85）
    ///
    /// 当 `used / total >= prompt_warn_threshold` 时，预算状态段标题从"预算状态"
    /// 升级为"⚠️ 预算告警"，提醒 LLM 容量压力大，可能需要主动整理认知。
    /// 范围 [0.0, 1.0]，建议 0.75~0.95。
    #[serde(default = "default_prompt_warn_threshold")]
    pub prompt_warn_threshold: f64,
}

/// 认知反馈关键阈值
#[derive(Debug, Clone, Serialize, Deserialize, Copy)]
pub struct CognitionThresholds {
    #[serde(default = "default_belief_flush_threshold")]
    pub belief_flush_threshold: usize,
    #[serde(default = "default_conflict_cache_capacity")]
    pub conflict_cache_capacity: usize,
    #[serde(default = "default_belief_boost_per_use")]
    pub belief_boost_per_use: f64,
    #[serde(default = "default_belief_ceiling")]
    pub belief_ceiling: f64,
}

fn default_belief_flush_threshold() -> usize {
    64
}
fn default_conflict_cache_capacity() -> usize {
    4096
}
fn default_belief_boost_per_use() -> f64 {
    0.02
}
fn default_belief_ceiling() -> f64 {
    0.99
}

impl Default for CognitionThresholds {
    fn default() -> Self {
        Self {
            belief_flush_threshold: default_belief_flush_threshold(),
            conflict_cache_capacity: default_conflict_cache_capacity(),
            belief_boost_per_use: default_belief_boost_per_use(),
            belief_ceiling: default_belief_ceiling(),
        }
    }
}

fn default_storage_backend() -> StorageBackendType {
    StorageBackendType::Mindscape
}
fn default_base_backend() -> StorageBackendType {
    StorageBackendType::Dir
}
fn default_storage_format() -> StorageFormat {
    StorageFormat::Yaml
}
fn default_max_entries() -> usize {
    1000
}
fn default_categories() -> Vec<String> {
    vec![
        "preference".to_string(),
        "fact".to_string(),
        "instruction".to_string(),
    ]
}
fn default_prompt_budget_tokens() -> usize {
    3500
}
fn default_prompt_overhead_tokens() -> usize {
    500
}
fn default_prompt_warn_threshold() -> f64 {
    0.85
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            storage_backend: default_storage_backend(),
            base_backend: default_base_backend(),
            storage_format: default_storage_format(),
            max_entries: default_max_entries(),
            categories: default_categories(),
            backend_config: serde_json::Value::Object(serde_json::Map::new()),
            cognition: CognitionThresholds::default(),
            prompt_budget_tokens: default_prompt_budget_tokens(),
            prompt_overhead_tokens: default_prompt_overhead_tokens(),
            prompt_warn_threshold: default_prompt_warn_threshold(),
        }
    }
}
