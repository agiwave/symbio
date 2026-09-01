//! 统一错误处理和工具函数
//!
//! 提供通用的错误转换和锁操作辅助功能

use std::sync::{RwLockReadGuard, RwLockWriteGuard};

/// 插件错误类型
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("插件未找到：{0}")]
    NotFound(String),
    #[error("插件调用未实现")]
    NotImplemented,
    #[error("输入验证失败：{0}")]
    ValidationError(String),
    #[error("内部错误：{0}")]
    InternalError(String),
    #[error("解析错误：{0}")]
    ParseError(String),
    #[error("请求超时")]
    Timeout,
    #[error("操作被拒绝：{0}")]
    Forbidden(String),
    #[error("操作中止")]
    Aborted,
    #[error("上下文丢失，需要重试")]
    RetryWithoutContextId,
    #[error("流解析错误：{0}")]
    StreamError(String),
    #[error("压缩失败")]
    CompressionFailed,
}

impl PluginError {
    /// 获取机器可读的错误码
    pub fn code(&self) -> &'static str {
        match self {
            PluginError::NotFound(_) => "NOT_FOUND",
            PluginError::NotImplemented => "NOT_IMPLEMENTED",
            PluginError::ValidationError(_) => "VALIDATION_ERROR",
            PluginError::InternalError(_) => "INTERNAL_ERROR",
            PluginError::ParseError(_) => "PARSE_ERROR",
            PluginError::Timeout => "TIMEOUT",
            PluginError::Forbidden(_) => "FORBIDDEN",
            PluginError::Aborted => "ABORTED",
            PluginError::RetryWithoutContextId => "RETRY_WITHOUT_CONTEXT_ID",
            PluginError::StreamError(_) => "STREAM_ERROR",
            PluginError::CompressionFailed => "COMPRESSION_FAILED",
        }
    }

    /// 将 PluginError 转换为通用的传输帧
    pub fn to_frame(&self) -> crate::symbio_core::PluginFrame {
        crate::symbio_core::PluginFrame::Error(
            self.to_string(),
            Some(serde_json::json!({
                "code": self.code()
            })),
        )
    }
}

/// 插件调用结果
pub type InvokeResponse<T> = Result<T, PluginError>;

impl From<serde_json::Error> for PluginError {
    fn from(err: serde_json::Error) -> Self {
        Self::ParseError(err.to_string())
    }
}

impl From<std::io::Error> for PluginError {
    fn from(err: std::io::Error) -> Self {
        Self::InternalError(err.to_string())
    }
}

/// 锁操作结果类型别名
pub type LockResult<T> = std::sync::LockResult<T>;

/// 将 PoisonError 转换为 PluginError
#[inline]
pub fn map_poison_error(e: std::sync::PoisonError<()>) -> PluginError {
    PluginError::InternalError(format!("Lock poisoned: {e}"))
}

/// 将 RwLock 读写锁错误转换为 PluginError
#[inline]
pub fn from_rwlock_write_error<T>(
    e: std::sync::TryLockError<RwLockWriteGuard<'_, T>>,
) -> PluginError {
    match e {
        std::sync::TryLockError::Poisoned(_) => {
            PluginError::InternalError("Write lock poisoned".to_string())
        }
        std::sync::TryLockError::WouldBlock => {
            PluginError::InternalError("Write lock would block".to_string())
        }
    }
}

#[inline]
pub fn from_rwlock_read_error<T>(
    e: std::sync::TryLockError<RwLockReadGuard<'_, T>>,
) -> PluginError {
    match e {
        std::sync::TryLockError::Poisoned(_) => {
            PluginError::InternalError("Read lock poisoned".to_string())
        }
        std::sync::TryLockError::WouldBlock => {
            PluginError::InternalError("Read lock would block".to_string())
        }
    }
}

/// 转换 RwLock 写锁为 Result
#[inline]
pub fn rwlock_write<T>(lock: &std::sync::RwLock<T>) -> LockResult<RwLockWriteGuard<'_, T>> {
    lock.write()
}

/// 转换 RwLock 读锁为 Result
#[inline]
pub fn rwlock_read<T>(lock: &std::sync::RwLock<T>) -> LockResult<RwLockReadGuard<'_, T>> {
    lock.read()
}

/// 将 Option 转换为 PluginError
#[inline]
pub fn ok_or_plugin_error<T>(opt: Option<T>, msg: impl Into<String>) -> Result<T, PluginError> {
    opt.ok_or_else(|| PluginError::NotFound(msg.into()))
}

/// 通用错误转换：从 String 到 PluginError
#[inline]
pub fn into_plugin_error(s: String) -> PluginError {
    PluginError::InternalError(s)
}

/// 将 Box<dyn Error> 转换为 PluginError
#[inline]
pub fn from_boxed_error(e: Box<dyn std::error::Error + Send + Sync>) -> PluginError {
    PluginError::InternalError(e.to_string())
}
