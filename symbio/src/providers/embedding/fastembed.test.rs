//! fastembed.rs 单元测试
//!
//! 对应源文件: `fastembed.rs`

use super::*;

/// `NoopEmbeddingService` 应稳定返回 None
#[tokio::test]
async fn test_noop_returns_none() {
    let svc = NoopEmbeddingService;
    assert!(svc.embed("anything").await.is_none());
}
