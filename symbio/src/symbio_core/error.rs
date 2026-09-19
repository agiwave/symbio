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
    #[error("请求频率受限：{0}")]
    RateLimited(String),
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

/// 机器可读错误码（跨插件边界的错误分类真源）
///
/// 错误跨插件边界传输时只能落到 `PluginFrame::Error(String, Option<Value>)`，
/// 分类信息由 `code` 字段承载，且"码"必须是单一类型化枚举：消费侧若回读该字段
/// 并与字面量（如 `"ABORTED"`）做字符串比较，文案/编码任一侧一改即静默失效。
///
/// 三段链路的职责划分：
/// - 生产侧：[`PluginError::code`] 返回 `ErrorCode`（编译器保证变体穷尽）；
/// - 传输侧：[`PluginError::to_frame`] 写入 `code.as_str()`；
/// - 消费侧：[`crate::symbio_core::PluginFrame::error_code`] 解析回 `ErrorCode`，
///   分派逻辑比较枚举值而非字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    NotFound,
    NotImplemented,
    ValidationError,
    InternalError,
    RateLimited,
    ParseError,
    Timeout,
    Forbidden,
    Aborted,
    RetryWithoutContextId,
    StreamError,
    CompressionFailed,
    /// 未知/缺失的码（对端版本超前或帧未携带 code 时的兜底，不参与等值分派）
    Unknown,
}

impl ErrorCode {
    /// 传输线上表示（`Error` 帧 `code` 字段的字面量）。
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::NotImplemented => "NOT_IMPLEMENTED",
            ErrorCode::ValidationError => "VALIDATION_ERROR",
            ErrorCode::InternalError => "INTERNAL_ERROR",
            ErrorCode::RateLimited => "RATE_LIMITED",
            ErrorCode::ParseError => "PARSE_ERROR",
            ErrorCode::Timeout => "TIMEOUT",
            ErrorCode::Forbidden => "FORBIDDEN",
            ErrorCode::Aborted => "ABORTED",
            ErrorCode::RetryWithoutContextId => "RETRY_WITHOUT_CONTEXT_ID",
            ErrorCode::StreamError => "STREAM_ERROR",
            ErrorCode::CompressionFailed => "COMPRESSION_FAILED",
            ErrorCode::Unknown => "UNKNOWN",
        }
    }

    /// 从线上表示解析回枚举；无法识别时返回 [`ErrorCode::Unknown`]。
    pub fn from_code(code: &str) -> Self {
        match code {
            "NOT_FOUND" => ErrorCode::NotFound,
            "NOT_IMPLEMENTED" => ErrorCode::NotImplemented,
            "VALIDATION_ERROR" => ErrorCode::ValidationError,
            "INTERNAL_ERROR" => ErrorCode::InternalError,
            "RATE_LIMITED" => ErrorCode::RateLimited,
            "PARSE_ERROR" => ErrorCode::ParseError,
            "TIMEOUT" => ErrorCode::Timeout,
            "FORBIDDEN" => ErrorCode::Forbidden,
            "ABORTED" => ErrorCode::Aborted,
            "RETRY_WITHOUT_CONTEXT_ID" => ErrorCode::RetryWithoutContextId,
            "STREAM_ERROR" => ErrorCode::StreamError,
            "COMPRESSION_FAILED" => ErrorCode::CompressionFailed,
            _ => ErrorCode::Unknown,
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PluginError {
    /// 获取机器可读的错误码
    pub fn code(&self) -> ErrorCode {
        match self {
            PluginError::NotFound(_) => ErrorCode::NotFound,
            PluginError::NotImplemented => ErrorCode::NotImplemented,
            PluginError::ValidationError(_) => ErrorCode::ValidationError,
            PluginError::InternalError(_) => ErrorCode::InternalError,
            PluginError::RateLimited(_) => ErrorCode::RateLimited,
            PluginError::ParseError(_) => ErrorCode::ParseError,
            PluginError::Timeout => ErrorCode::Timeout,
            PluginError::Forbidden(_) => ErrorCode::Forbidden,
            PluginError::Aborted => ErrorCode::Aborted,
            PluginError::RetryWithoutContextId => ErrorCode::RetryWithoutContextId,
            PluginError::StreamError(_) => ErrorCode::StreamError,
            PluginError::CompressionFailed => ErrorCode::CompressionFailed,
        }
    }

    /// 是否为"用户主动中止"语义（非业务失败：不落错误事件、不冒泡给前端为 Error）。
    pub fn is_abort(&self) -> bool {
        matches!(self, PluginError::Aborted)
    }

    /// 将 PluginError 转换为通用的传输帧
    pub fn to_frame(&self) -> crate::symbio_core::PluginFrame {
        crate::symbio_core::PluginFrame::Error(
            self.to_string(),
            Some(serde_json::json!({
                "code": self.code().as_str()
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

// 曾有一个 `From<std::io::Error> for PluginError`（经 `to_string()` 收敛为
// `InternalError`）。它已删除，且**不要**加回来：
//
// - 它把 `io::ErrorKind` 抹平成文案，调用方再没法把"文件不存在"与"权限不足"
//   分开，而本文件的既定纪律恰恰是"分派只认类型化错误码，不认文案"；
// - 它在这套代码里**零可达路径**：所有 io 失败都在边界处显式
//   `.map_err(..)` 成各自正确的分类（如 `store.rs` 的"条目不存在"）。
//   保留一个无人使用、且用起来会丢信息的 impl，只会诱导后来者走错路。
//   真需要 `?` 传播 io 错误时，编译器会报出来，那正是应当停下来做分类决策的点。

/// 取读锁；**锁毒化时恢复数据，而不是二次 panic**。
///
/// 为什么是恢复：这些锁保护的都是普通表（`envs` / `extensions` / 内存条目表），
/// 毒化只可能来自"持锁线程 panic"这一种情形，表本身依旧结构完好。此时若跟着
/// panic，一个线程的崩溃就把整个插件永久变成不可用——代价远大于收益。
///
/// 可见性刻意是 `pub(crate)`：写成 `pub` 会被本模块的 `pub use error::*`
/// 出口成对外 API，`dead_code` 便对它结构性失明，最终攒出一批"无人调用的
/// 词汇表"（本文件曾有一版 8 个这样的 helper，全仓零使用）。`pub(crate)`
/// 让编译器继续盯着：一旦没人用了，它会直接报出来。
#[inline]
pub(crate) fn lock_read<T>(lock: &std::sync::RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 取写锁；毒化时恢复（理由见 [`lock_read`]）。
#[inline]
pub(crate) fn lock_write<T>(lock: &std::sync::RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::PluginFrame;

    /// 每个变体的错误码经 `to_frame` → `error_code` 往返后保持一致
    /// （分派依据必须是类型化错误码，而非文案字面量）。
    #[test]
    fn error_code_roundtrip_through_frame() {
        let cases = [
            (PluginError::NotFound("x".into()), ErrorCode::NotFound),
            (PluginError::NotImplemented, ErrorCode::NotImplemented),
            (
                PluginError::ValidationError("x".into()),
                ErrorCode::ValidationError,
            ),
            (
                PluginError::InternalError("x".into()),
                ErrorCode::InternalError,
            ),
            (PluginError::RateLimited("x".into()), ErrorCode::RateLimited),
            (PluginError::ParseError("x".into()), ErrorCode::ParseError),
            (PluginError::Timeout, ErrorCode::Timeout),
            (PluginError::Forbidden("x".into()), ErrorCode::Forbidden),
            (PluginError::Aborted, ErrorCode::Aborted),
            (
                PluginError::RetryWithoutContextId,
                ErrorCode::RetryWithoutContextId,
            ),
            (PluginError::StreamError("x".into()), ErrorCode::StreamError),
            (PluginError::CompressionFailed, ErrorCode::CompressionFailed),
        ];
        for (err, want) in cases {
            let frame = err.to_frame();
            assert_eq!(frame.error_code(), Some(want), "往返失败：{err:?}");
            // 生产侧 code() 与帧内 code 必须一致（单一真源）
            assert_eq!(err.code(), want);
        }
    }

    /// abort 分派只认错误码，不受文案影响（防止文案改动静默失效）。
    #[test]
    fn abort_dispatch_ignores_message_text() {
        let frame = PluginError::Aborted.to_frame();
        assert!(frame.is_abort());
        // 非 abort 错误即使文案含 "abort" 字样也不应被判为中止
        let decoy = PluginFrame::Error(
            "用户手动中止了本次回复".into(),
            Some(serde_json::json!({ "code": "INTERNAL_ERROR" })),
        );
        assert!(!decoy.is_abort());
    }

    /// 不可识别/缺失的 code 降级为 None，不误判为任何具体语义。
    #[test]
    fn unknown_or_missing_code_is_none() {
        assert_eq!(ErrorCode::from_code("NO_SUCH_CODE"), ErrorCode::Unknown);
        let frame = PluginFrame::Error("x".into(), Some(serde_json::json!({"code": "FUTURE"})));
        assert_eq!(frame.error_code(), None);
        assert_eq!(PluginFrame::Error("x".into(), None).error_code(), None);
        assert_eq!(PluginFrame::Data(serde_json::json!(1)).error_code(), None);
    }

    /// `is_abort` 谓词与错误码保持一致。
    #[test]
    fn is_abort_predicate_matches_code() {
        assert!(PluginError::Aborted.is_abort());
        assert!(!PluginError::Timeout.is_abort());
        assert!(!PluginError::RetryWithoutContextId.is_abort());
    }

    /// 锁被毒化（持锁线程 panic）后，取锁辅助仍能拿回数据——而不是跟着 panic
    /// 把整个插件带走。这是 `lock_read` / `lock_write` 存在的唯一理由。
    #[test]
    fn lock_helpers_recover_from_poisoning() {
        let lock = std::sync::RwLock::new(vec![1, 2, 3]);

        // 在另一线程持写锁并 panic ⇒ 毒化
        let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lock.write().unwrap();
            panic!("持锁崩溃，制造毒化");
        }));
        assert!(died.is_err(), "前提：该线程应当 panic 掉");
        assert!(lock.read().is_err(), "前提：锁此时确实已被毒化");

        // 常规写法会在这里二次 panic；我们的辅助应当恢复数据并继续
        assert_eq!(*lock_read(&lock), vec![1, 2, 3], "毒化后数据应完好可取");
        lock_write(&lock).push(4);
        assert_eq!(*lock_read(&lock), vec![1, 2, 3, 4], "毒化后仍可正常写入");
    }
}
