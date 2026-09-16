//! 模型服务限流器 —— 按 provider_id 记录"上次发起请求的时间"的最小请求间隔节流。
//!
//! Phase sink：自 `symbio_core/rate_limit.rs` 下沉至 session 插件——E-② 后
//! 该限流器的唯一消费者是 session 编排层（`orchestrator.rs`，发起 LLM 请求前
//! 节流），属单一模块私有设施，不再置于 core 共享层。

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// 限流器：按 provider_id 记录"上次发起请求的时间"
///
/// 设计原则：
/// - 进程内单例（每个 Provider 一个时间戳）
/// - 加锁粒度：per-provider 共用一把互斥但仅做 map 存取，等待通过
///   独立的 sleep 完成，Provider A 的限流不会阻塞 Provider B 的放行
#[derive(Default)]
pub struct ProviderRateLimiter {
    last_request: Mutex<HashMap<String, Instant>>,
}

/// 进程内全局限流器单例
///
/// 会话编排层发起模型请求前调用：
/// `super::rate_limit::RATE_LIMITER
///     .wait(&entry.provider_id, entry.rate_limit_ms).await;`
pub static RATE_LIMITER: LazyLock<ProviderRateLimiter> =
    LazyLock::new(ProviderRateLimiter::default);

impl ProviderRateLimiter {
    /// 阻塞等待直到距上次请求至少 `min_interval_ms` 毫秒
    ///
    /// - `min_interval_ms == 0` 时直接放行
    /// - **首次请求立即放行**，仅记录时间戳；后续请求才按 `min_interval_ms` 节流
    pub async fn wait(&self, provider_id: &str, min_interval_ms: u64) {
        if min_interval_ms == 0 {
            return;
        }
        let interval = Duration::from_millis(min_interval_ms);
        loop {
            let now = Instant::now();
            // 计算本次需要 sleep 的时长（None = 立即放行）
            let sleep_for: Option<Duration> = {
                let mut map = self.last_request.lock().await;
                match map.get(provider_id) {
                    None => {
                        // 首次请求：立即放行，仅记录时间戳供后续节流
                        map.insert(provider_id.to_string(), now);
                        None
                    }
                    Some(&last) => {
                        let elapsed = now.saturating_duration_since(last);
                        if elapsed >= interval {
                            // 已超过最小间隔：放行并刷新时间戳
                            map.insert(provider_id.to_string(), now);
                            None
                        } else {
                            Some(interval - elapsed)
                        }
                    }
                }
            };
            match sleep_for {
                None => return,
                Some(remaining) => {
                    // 提前 5ms 唤醒避免睡过头；若剩余时间已 < 5ms 则直接重试
                    let sleep = remaining.saturating_sub(Duration::from_millis(5));
                    if sleep.is_zero() {
                        continue;
                    }
                    tokio::time::sleep(sleep).await;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "rate_limit.test.rs"]
mod tests;
