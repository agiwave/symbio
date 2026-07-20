use crate::symbio_core::PluginError;
use thiserror::Error;

use super::store::StoreError;

/// 智能体插件错误枚举
///
/// 设计原则：
/// - 错误按"业务语义"分类，而非按"技术位置"
/// - 携带上下文字符串，便于前端展示
/// - 通过 `From<AgentError> for PluginError` 精细映射为前端可识别的 HTTP-like 状态
#[derive(Debug, Error)]
pub enum AgentError {
    /// Agent 元数据/Profile 错误（找不到/格式错误/创建失败）
    #[error("Profile error: {0}")]
    Profile(String),

    /// Mindscape 引擎错误（运行期/状态错误）
    #[error("Mindscape error: {0}")]
    Mindscape(String),

    /// 存储层错误（已聚合底层 IO/解析错误）
    #[error("Storage error: {0}")]
    Storage(String),

    /// 配置/参数错误（用户输入问题）
    #[error("Configuration error: {0}")]
    Config(String),

    /// 嵌入服务错误
    #[error("Embedding error: {0}")]
    Embedding(String),

    /// 校验错误（请求参数不合法）
    #[error("Validation error: {0}")]
    Validation(String),

    /// 能力调用错误
    #[error("Capability error: {0}")]
    Capability(String),

    /// 推理错误
    #[error("Reasoning error: {0}")]
    Reasoning(String),

    /// 学习错误
    #[error("Learning error: {0}")]
    Learning(String),

    /// 规划错误
    #[error("Planning error: {0}")]
    Planning(String),

    /// 底层 IO 错误
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON 序列化/反序列化错误
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// YAML 序列化/反序列化错误
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yml::Error),

    /// 资源未找到
    #[error("Not found: {0}")]
    NotFound(String),

    /// 资源已存在（冲突）
    #[error("Already exists: {0}")]
    AlreadyExists(String),

    /// 未知错误
    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl AgentError {
    /// 构造 Profile 错误
    pub fn profile(msg: impl Into<String>) -> Self {
        Self::Profile(msg.into())
    }

    /// 构造 Validation 错误
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }

    /// 构造 Capability 错误
    pub fn capability(msg: impl Into<String>) -> Self {
        Self::Capability(msg.into())
    }

    /// 构造 Reasoning 错误
    pub fn reasoning(msg: impl Into<String>) -> Self {
        Self::Reasoning(msg.into())
    }

    /// 构造 Learning 错误
    pub fn learning(msg: impl Into<String>) -> Self {
        Self::Learning(msg.into())
    }

    /// 构造 Planning 错误
    pub fn planning(msg: impl Into<String>) -> Self {
        Self::Planning(msg.into())
    }

    /// 构造 NotFound 错误
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    /// 构造 AlreadyExists 错误
    pub fn already_exists(msg: impl Into<String>) -> Self {
        Self::AlreadyExists(msg.into())
    }

    /// 构造 Unknown 错误
    pub fn unknown(msg: impl Into<String>) -> Self {
        Self::Unknown(msg.into())
    }

    /// 判断错误是否属于"找不到资源"类
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }

    /// 判断错误是否属于"用户输入非法"类
    pub fn is_user_error(&self) -> bool {
        matches!(
            self,
            Self::Validation(_) | Self::Config(_) | Self::Capability(_)
        )
    }

    /// 获取错误分类代码
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Profile(_) => "AGENT_PROFILE_ERROR",
            Self::Mindscape(_) => "AGENT_MINDSCAPE_ERROR",
            Self::Storage(_) => "AGENT_STORAGE_ERROR",
            Self::Config(_) => "AGENT_CONFIG_ERROR",
            Self::Embedding(_) => "AGENT_EMBEDDING_ERROR",
            Self::Validation(_) => "AGENT_VALIDATION_ERROR",
            Self::Capability(_) => "AGENT_CAPABILITY_ERROR",
            Self::Reasoning(_) => "AGENT_REASONING_ERROR",
            Self::Learning(_) => "AGENT_LEARNING_ERROR",
            Self::Planning(_) => "AGENT_PLANNING_ERROR",
            Self::Io(_) => "AGENT_IO_ERROR",
            Self::Json(_) => "AGENT_JSON_ERROR",
            Self::Yaml(_) => "AGENT_YAML_ERROR",
            Self::NotFound(_) => "AGENT_NOT_FOUND",
            Self::AlreadyExists(_) => "AGENT_ALREADY_EXISTS",
            Self::Unknown(_) => "AGENT_UNKNOWN_ERROR",
        }
    }

    /// 获取错误级别
    pub fn error_level(&self) -> ErrorLevel {
        match self {
            Self::Validation(_)
            | Self::Config(_)
            | Self::Capability(_)
            | Self::NotFound(_)
            | Self::AlreadyExists(_) => ErrorLevel::Warning,
            Self::Profile(_) | Self::Storage(_) | Self::Mindscape(_) => ErrorLevel::Error,
            Self::Embedding(_) | Self::Reasoning(_) | Self::Learning(_) | Self::Planning(_) => {
                ErrorLevel::Error
            },
            Self::Io(_) | Self::Json(_) | Self::Yaml(_) => ErrorLevel::Error,
            Self::Unknown(_) => ErrorLevel::Error,
        }
    }
}

/// 错误级别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorLevel {
    /// 警告级别
    Warning,
    /// 错误级别
    Error,
}

/// 统一错误桥接：将存储层错误转换为 AgentError
///
/// I-048: 直接映射到语义化变体，避免字符串模式匹配。
impl From<StoreError> for AgentError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::AlreadyExists(msg) => AgentError::AlreadyExists(msg),
            StoreError::NotFound(msg) => AgentError::NotFound(msg),
            StoreError::InvalidInput(msg) => AgentError::Validation(msg),
            StoreError::Backend(msg) => AgentError::Storage(msg),
            StoreError::NotSupported(msg) => AgentError::Storage(format!("NotSupported: {}", msg)),
        }
    }
}

/// 将 AgentError 映射为 PluginError，**保留语义**而非全部降级为 InternalError
///
/// I-048: 使用变体匹配替代字符串模式匹配。
///
/// 映射策略：
/// - `Validation` / `Config` / `NotFound` / `AlreadyExists` => `PluginError::ValidationError`
/// - 其它 => `PluginError::InternalError`（5xx 等价）
impl From<AgentError> for PluginError {
    fn from(err: AgentError) -> Self {
        let msg = err.to_string();

        match &err {
            // 用户输入类错误 -> ValidationError
            AgentError::Validation(_)
            | AgentError::Config(_)
            | AgentError::NotFound(_)
            | AgentError::AlreadyExists(_) => PluginError::ValidationError(msg),
            // 其余视为内部错误
            _ => PluginError::InternalError(msg),
        }
    }
}

pub type AgentResult<T> = Result<T, AgentError>;

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
