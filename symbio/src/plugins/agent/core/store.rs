use super::CognitiveUnit;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum StoreError {
    AlreadyExists(String),
    NotFound(String),
    InvalidInput(String),
    Backend(String),
    /// 显式标记：当前 store 不支持某操作（如 search）
    NotSupported(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyExists(msg) => write!(f, "AlreadyExists: {msg}"),
            Self::NotFound(msg) => write!(f, "NotFound: {msg}"),
            Self::InvalidInput(msg) => write!(f, "InvalidInput: {msg}"),
            Self::Backend(msg) => write!(f, "Backend: {msg}"),
            Self::NotSupported(msg) => write!(f, "NotSupported: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// 稳定错误码
///
/// 用途：日志聚合 / 监控告警 / 多语言错误处理时按 code 分类，
/// 避免依赖 `Display` 输出（可能含动态字符串）。
///
/// 格式：`S-NNN`
/// - S = Store 域
/// - NNN = 3 位数字（便于扩展）
impl StoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AlreadyExists(_) => "S-001",
            Self::NotFound(_) => "S-002",
            Self::InvalidInput(_) => "S-003",
            Self::Backend(_) => "S-004",
            Self::NotSupported(_) => "S-005",
        }
    }

    /// 分类（用于 metrics label）
    pub fn category(&self) -> &'static str {
        match self {
            Self::AlreadyExists(_) | Self::NotFound(_) | Self::InvalidInput(_) => "user_error",
            Self::Backend(_) | Self::NotSupported(_) => "system_error",
        }
    }
}

impl From<String> for StoreError {
    fn from(s: String) -> Self {
        Self::Backend(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FilterExpr {
    Eq {
        key: String,
        value: Value,
    },
    Ne {
        key: String,
        value: Value,
    },
    Gt {
        key: String,
        value: f64,
    },
    Gte {
        key: String,
        value: f64,
    },
    Lt {
        key: String,
        value: f64,
    },
    Lte {
        key: String,
        value: f64,
    },
    In {
        key: String,
        values: Vec<Value>,
    },
    Contains {
        key: String,
        substring: String,
    },
    StartsWith {
        key: String,
        prefix: String,
    },
    Relation {
        key: String,
        value: String,
    },
    /// 语义搜索（需 embedding 支持，不支持时可降级为全文搜索）
    Semantic {
        query: String,
        min_score: f32,
    },
    And(Vec<FilterExpr>),
    Or(Vec<FilterExpr>),
    Not(Box<FilterExpr>),
}

impl FilterExpr {
    pub fn eq(key: impl Into<String>, value: Value) -> Self {
        Self::Eq {
            key: key.into(),
            value,
        }
    }
    /// `is_a` 是关系的一种，返回 `Relation { key: "is_a", value }`
    pub fn is_a(value: impl Into<String>) -> Self {
        Self::Relation {
            key: "is_a".to_string(),
            value: value.into(),
        }
    }
    #[allow(dead_code)]
    pub fn and(exprs: Vec<FilterExpr>) -> Self {
        Self::And(exprs)
    }
    /// 永远匹配的过滤器（用于"查询全部"场景，语义比 `And(vec![])` 清晰）
    ///
    /// 替代之前使用 `FilterExpr::And(vec![])` 作为"匹配所有"的黑客写法
    pub fn match_all() -> Self {
        // And(vec![]) 的 evaluate_filter 结果是 true（empty `all()`），保留实现但提供语义化入口
        Self::And(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SortKey {
    Property(String),
    Relevance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRequest {
    pub offset: usize,
    pub limit: usize,
    pub sort_by: Option<SortKey>,
    pub sort_desc: bool,
}

impl PageRequest {
    pub fn new(offset: usize, limit: usize) -> Self {
        Self {
            offset,
            limit,
            sort_by: None,
            sort_desc: false,
        }
    }
    pub fn first(limit: usize) -> Self {
        Self {
            offset: 0,
            limit,
            sort_by: None,
            sort_desc: false,
        }
    }
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 20,
            sort_by: None,
            sort_desc: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResult {
    pub items: Vec<CognitiveUnit>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub scores: Option<Vec<f32>>,
}

impl PageResult {
    #[allow(dead_code)]
    pub fn has_next(&self) -> bool {
        self.offset + self.limit < self.total
    }
}

/// 认知单元存储抽象
///
/// ## 关键设计：能力查询契约
///
/// `search()` 是**可选能力**——基础后端（file/dir/memory/sqlite）不实现向量检索，
/// 由装饰器 `EmbeddingStore` 提供语义检索。
///
/// 之前的设计缺陷：所有 4 个后端的 `search` 都返回 `Ok(Vec::new())` 桩，
/// 上层无法区分"真没找到"和"后端不支持"，造成静默 bug。
///
/// 现在：
/// - 新增 `is_search_supported()` 默认 `false`
/// - 基础后端保持 `search` 桩但**通过 trait 方法明确声明不支持**
/// - 上层（如 `EmbeddingStore`）可重写 `is_search_supported() = true`
/// - 上层调用方（如 `MindscapeScaffold::search`）应先检查该方法
#[async_trait::async_trait]
pub trait AgentStore: Send + Sync {
    // ── 基础 CRUD ──
    async fn get(&self, id: &str) -> Result<Option<CognitiveUnit>, StoreError>;
    async fn insert(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError>;
    async fn update(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError>;
    async fn upsert(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError>;
    async fn delete(&self, id: &str) -> Result<bool, StoreError>;

    /// 统一查询：结构化过滤 + 语义搜索 + 计数
    ///
    /// - `filter` 支持所有 FilterExpr 变体（含 `Semantic`）
    /// - `PageResult.total` 始终返回 store 中匹配总数（不受分页影响）
    /// - `PageResult.scores` 仅在 Semantic 过滤时有值
    /// - `count` 查询：`query(&FilterExpr::match_all(), &PageRequest::first(0))` 后读 `total`
    async fn query(
        &self,
        filter: &FilterExpr,
        page: &PageRequest,
    ) -> Result<PageResult, StoreError>;

    fn cancel_background_tasks(&self) {}

    async fn insert_batch(&self, units: &[CognitiveUnit]) -> Result<usize, StoreError> {
        let mut count = 0;
        for unit in units {
            self.insert(unit).await?;
            count += 1;
        }
        Ok(count)
    }

    // ── 认知反馈与生命周期 ──

    /// 记录认知单元被访问（认知反馈，默认 no-op）
    async fn record_access(&self, _unit_ids: &[&str]) {}

    /// 优雅关闭引擎（刷新 belief buffer、取消后台任务）
    async fn shutdown(&self) {}
}

pub fn evaluate_filter(unit: &CognitiveUnit, expr: &FilterExpr) -> bool {
    match expr {
        FilterExpr::Eq { key, value } => unit.get(key) == Some(value),
        FilterExpr::Ne { key, value } => unit.get(key) != Some(value),
        FilterExpr::Gt { key, value } => unit
            .get(key)
            .and_then(|v| v.as_f64())
            .is_some_and(|v| v > *value),
        FilterExpr::Gte { key, value } => unit
            .get(key)
            .and_then(|v| v.as_f64())
            .is_some_and(|v| v >= *value),
        FilterExpr::Lt { key, value } => unit
            .get(key)
            .and_then(|v| v.as_f64())
            .is_some_and(|v| v < *value),
        FilterExpr::Lte { key, value } => unit
            .get(key)
            .and_then(|v| v.as_f64())
            .is_some_and(|v| v <= *value),
        FilterExpr::In { key, values } => unit.get(key).is_some_and(|v| values.contains(v)),
        FilterExpr::Contains { key, substring } => unit
            .get(key)
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.to_lowercase().contains(&substring.to_lowercase())),
        FilterExpr::StartsWith { key, prefix } => unit
            .get(key)
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.starts_with(prefix.as_str())),
        FilterExpr::Relation {
            key: relation,
            value,
        } => {
            // 通用关系过滤（包括 is_a）
            let matches_str = |target: &str| -> bool {
                if value.ends_with("::*") {
                    let prefix = &value[..value.len() - 2];
                    target.starts_with(prefix)
                } else {
                    target == value
                }
            };
            // matches_str 接 &str，
            // &String 通过 deref coercion 自动转 &str，不需要 `&**` 显式解引用
            unit.relations(relation).iter().any(|s| matches_str(s))
        }
        // Semantic 无法在内存中求值，由 store 实现自行处理
        FilterExpr::Semantic { .. } => true,
        FilterExpr::And(exprs) => exprs.iter().all(|e| evaluate_filter(unit, e)),
        FilterExpr::Or(exprs) => exprs.iter().any(|e| evaluate_filter(unit, e)),
        FilterExpr::Not(expr) => !evaluate_filter(unit, expr),
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
