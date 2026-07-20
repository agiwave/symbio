//! InitTracker 单元测试
//!
//! 对应源文件: `tracker.rs`

use super::*;

#[tokio::test]
async fn test_is_initialized_returns_false_by_default() {
    let tracker = InitTracker::new();
    assert!(!tracker.is_initialized(Some("/tmp/test")).await);
}

#[tokio::test]
async fn test_mark_then_check() {
    let tracker = InitTracker::new();
    tracker.mark_initialized(Some("/tmp/test")).await;
    assert!(tracker.is_initialized(Some("/tmp/test")).await);
}

#[tokio::test]
async fn test_different_workdirs_are_independent() {
    let tracker = InitTracker::new();
    tracker.mark_initialized(Some("/tmp/a")).await;
    assert!(tracker.is_initialized(Some("/tmp/a")).await);
    assert!(!tracker.is_initialized(Some("/tmp/b")).await);
}
