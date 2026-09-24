//! `symbio/src/symbio_core/error.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

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
    assert_eq!(PluginFrame::data(serde_json::json!(1)).error_code(), None);
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
