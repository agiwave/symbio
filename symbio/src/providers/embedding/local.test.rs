//! local.rs 单元测试
//!
//! 对应源文件: `local.rs`
//!
//! 数值对齐验证（tract vs fastembed 余弦相似度 0.999558）在迁移 spike 阶段
//! 已实测通过并记录于 `docs/DECISIONS.md` ADR-014；fastembed 依赖本体已移除，
//! 无法在单测中再跑参考管线。

use super::*;

/// `NoopEmbeddingService` 应稳定返回 None
#[tokio::test]
async fn test_noop_returns_none() {
    let svc = NoopEmbeddingService;
    assert!(svc.embed("anything").await.is_none());
}
