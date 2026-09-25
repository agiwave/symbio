//! VDFS 错误：机器可读码、字段级校验载荷与便捷构造。

use serde::{Deserialize, Serialize};

// ==================== 错误 ====================

/// 字段级校验错误（provider 在 `write` 中校验后返回，消费者据此逐字段高亮）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VdfsFieldError {
    /// 字段键
    pub field: String,
    /// 人类可读原因
    pub message: String,
}

/// 校验失败载荷
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VdfsValidationError {
    pub message: String,
    #[serde(default)]
    pub fields: Vec<VdfsFieldError>,
}

impl VdfsValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fields: Vec::new(),
        }
    }

    pub fn with_field(mut self, field: impl Into<String>, message: impl Into<String>) -> Self {
        self.fields.push(VdfsFieldError {
            field: field.into(),
            message: message.into(),
        });
        self
    }

    /// 是否已记录字段级错误
    pub fn has_fields(&self) -> bool {
        !self.fields.is_empty()
    }
}

/// VDFS 操作结果（错误类型由协议自身定义，不依赖任何宿主错误体系）
pub type VdfsResult<T> = Result<T, VdfsError>;

/// VDFS 错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VdfsError {
    /// 节点 / 挂载点不存在
    NotFound(String),
    /// 该 provider 未实现此操作
    NotImplemented,
    /// 校验失败（可携带字段级错误）
    Invalid(VdfsValidationError),
    /// 访问位不允许（无 `r` / `w` 等）
    Forbidden(String),
    /// 冲突（并发 / 已存在 / 非空目录）
    Conflict(String),
    /// 内部错误
    Internal(String),
}

impl std::fmt::Display for VdfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) => write!(f, "节点不存在：{m}"),
            Self::NotImplemented => write!(f, "该操作未实现"),
            Self::Invalid(e) => write!(f, "校验失败：{}", e.message),
            Self::Forbidden(m) => write!(f, "操作被拒绝：{m}"),
            Self::Conflict(m) => write!(f, "冲突：{m}"),
            Self::Internal(m) => write!(f, "内部错误：{m}"),
        }
    }
}

impl std::error::Error for VdfsError {}

impl VdfsError {
    /// 机器可读码（跨边界传输时使用，避免字符串比较）
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "NOT_FOUND",
            Self::NotImplemented => "NOT_IMPLEMENTED",
            Self::Invalid(_) => "VALIDATION_ERROR",
            Self::Forbidden(_) => "FORBIDDEN",
            Self::Conflict(_) => "CONFLICT",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }

    /// 便捷构造校验错误
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(VdfsValidationError::new(message))
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// 是否为「未实现」——宿主据此在能力判定中隐藏对应入口
    pub fn is_not_implemented(&self) -> bool {
        matches!(self, Self::NotImplemented)
    }
}
