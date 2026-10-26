//! `rate_limit` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `rate_limit.rs` 只保留生产代码，测试全部放本文件。

use super::*;

#[tokio::test]
async fn zero_interval_passes_immediately() {
    let limiter = ProviderRateLimiter::default();
    let start = Instant::now();
    limiter.wait("p0", 0).await;
    limiter.wait("p0", 0).await;
    assert!(start.elapsed().as_millis() < 50);
}

#[tokio::test]
async fn first_request_passes_then_throttles() {
    let limiter = ProviderRateLimiter::default();
    let start = Instant::now();
    limiter.wait("p1", 50).await;
    assert!(start.elapsed().as_millis() < 50, "首次请求应立即放行");

    limiter.wait("p1", 50).await;
    assert!(
        start.elapsed().as_millis() >= 40,
        "第二次请求应等待最小间隔"
    );

    // 不同 provider 互不影响
    let p2_start = Instant::now();
    limiter.wait("p2", 50).await;
    assert!(
        p2_start.elapsed().as_millis() < 40,
        "Provider B 不受 A 限流影响"
    );
}
