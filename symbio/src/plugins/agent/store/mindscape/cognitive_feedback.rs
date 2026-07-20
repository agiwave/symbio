//! 认知反馈模块
//!
//! 实现认知闭环：推理结果反馈到知识库，形成"使用→验证→更新"循环。
//!
//! 核心机制：
//! 1. **使用反馈**：被检索/引用的认知单元，其 meta_belief 自动提升
//! 2. **置信度衰减**：长期未被引用的知识，confidence 自然下降（由 memory_consolidation 模块处理）

use crate::plugins::agent::core::AgentStore;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use tracing::instrument;

/// 认知反馈引擎
#[derive(Clone)]
pub struct CognitiveFeedback {
    store: Arc<dyn AgentStore>,
    thresholds: crate::plugins::agent::core::CognitionThresholds,
    pub(crate) belief_buffer: Arc<AsyncMutex<std::collections::HashMap<String, u32>>>,
}

/// v25-N2 + I-005: belief 攒批触发阈值
///
/// 默认 64（≈ 一次 IO 批量写 64 行），由 `CognitionThresholds::default` 提供。
/// 当 buffer 中不同 unit 数 ≥ 此值时**同步**触发一次 flush。
///
/// 冲突去重缓存容量上限（+ I-005）
/// 原本是 `const CONFLICT_CACHE_CAPACITY = 4096`，v28 改为可配置。
/// 默认值在 [`crate::plugins::agent::core::CognitionThresholds::default`]。
/// 防止高频 detect_conflicts 场景下 HashSet 无限增长。
/// 达到上限时清空，让 store 重新成为 source of truth。
///
impl CognitiveFeedback {
    /// 构造 CognitiveFeedback
    pub fn new(store: Arc<dyn AgentStore>) -> Self {
        Self {
            store,
            thresholds: crate::plugins::agent::core::CognitionThresholds::default(),
            belief_buffer: Arc::new(AsyncMutex::new(std::collections::HashMap::with_capacity(
                128,
            ))),
        }
    }

    /// I-005 / S-3: 注入自定义阈值
    ///
    /// `MindscapeScaffold::new` 从 `AgentConfig::cognition` 读取阈值后调用本方法。
    /// 老调用方继续用 `CognitiveFeedback::new`（默认阈值，与 v27 行为一致）。
    pub fn with_thresholds(
        mut self,
        thresholds: crate::plugins::agent::core::CognitionThresholds,
    ) -> Self {
        self.thresholds = thresholds;
        self
    }

    /// 使用反馈：被检索到的认知单元，提升其 meta_belief
    ///
    /// 调用时机：search 返回结果后，由上层调用
    /// 策略：v25-N2 之前对每个被检索到的单元**立即**做 1 次 get + 1 次 update；
    ///       现在改为**攒批**——访问计数先累加到内存 buffer，
    ///       buffer 满（`BELIEF_BUFFER_FLUSH_THRESHOLD`）或显式 `flush_belief_buffer()` 时
    ///       再做 N 次 get + N 次 update。
    ///
    /// 性能：100 次访问同一单元：
    ///   * 修复前：100 get + 100 update = 200 IO
    ///   * 修复后：1 get + 1 update = 2 IO
    ///
    /// 退化场景：100 次访问不同单元（worst case）：
    ///   * 修复后：buffer 满自动 flush，仍然 100 get + 100 update，但 flush 是**集中**的
    ///     便于 transaction / batched IO 优化（v26+ 进一步做）
    ///
    /// 数据丢失容忍：进程崩溃时未 flush 的计数会丢失；
    /// meta_belief 是软指标（用于遗忘曲线/晋升），丢失可接受。
    pub async fn on_units_retrieved(&self, unit_ids: &[&str]) {
        // v26-N1: 临界区只做 in-memory 累加（无 .await），持锁时长几十纳秒
        let buffer_size = {
            let mut buf = self.belief_buffer.lock().await;
            for id in unit_ids {
                *buf.entry((*id).to_string()).or_insert(0) += 1;
            }
            buf.len()
        };

        // 阶段 2：达到阈值 → 自动 flush
        // 注意：tokio::sync::Mutex 的 guard 已在上面作用域结束时 drop，
        // 这里不持锁进入 flush
        // I-005 / S-3: 阈值可配置
        if buffer_size >= self.thresholds.belief_flush_threshold {
            self.flush_belief_buffer().await;
        }
    }

    /// 报告当前 buffer 中"待 flush" 的访问记录数
    ///
    /// 调用方退出前 peek buffer 长度，> 0 才触发 flush（避免空 buffer 的 IO 浪费）。
    /// 典型场景：`MindscapeScaffold::drop` 兜底逻辑。
    pub async fn pending_belief_updates(&self) -> usize {
        self.belief_buffer.lock().await.len()
    }

    /// v25-N2: 手动刷新 belief buffer
    ///
    /// 遍历 buffer，对每个 (id, count) 做：
    ///
    /// 1. `store.get(id)` 取当前值
    /// 2. `new_belief = current + count * BELIEF_BOOST_PER_USE`（clamp 到 ceiling）
    /// 3. 跳过 `is_a` 含 `kind` / `prop` / `meta` / `relation` / `cu` 的 schema 元数据（不修改本体）
    ///    和无变化项
    /// 4. `store.update(&unit)` 写一次
    ///
    /// 调用时机：
    ///   - 进程关闭前（shutdown 路径，v25-N3）
    ///   - 缓冲区满时（on_units_retrieved 自动触发）
    ///   - 测试 / 调试用 `flush_belief_buffer().await`
    ///
    /// v26-N1: **锁外**执行 IO（mem::take snapshot 后立即释放锁）
    ///
    /// 返回：成功 flush 的 unit 数（不含被跳过项）
    #[instrument(skip(self), fields(buffer_size))]
    pub async fn flush_belief_buffer(&self) -> usize {
        // 阶段 1：在锁内 take 出 buffer 快照并立即释放锁
        let snapshot: std::collections::HashMap<String, u32> = {
            let mut buf = self.belief_buffer.lock().await;
            std::mem::take(&mut *buf)
            // buf 在这里 drop，释放锁
        };
        if snapshot.is_empty() {
            return 0;
        }
        let mut flushed = 0usize;
        for (id, count) in snapshot {
            let Ok(Some(mut unit)) = self.store.get(&id).await else {
                // 单元已被删除 / 读失败 → 跳过（不重试）
                crate::plugin_warn!(
                    "agent",
                    "[CognitiveFeedback] flush: unit '{}' not found in store, skipping",
                    id
                );
                continue;
            };
            // 跳过 schema 元数据（is_a 含 kind/prop/meta/relation/cu）
            // 这些是 seed_cus.jsonl 定义的本体（认知 schema），不应被动态 belief 调整
            // LLM 可通过 save/delete 主动管理元数据，但 belief 是系统动态计算的——元数据不参与
            if let Some(is_a) = unit.get("is_a").and_then(|v| v.as_array()) {
                if is_a.iter().any(|v| {
                    matches!(
                        v.as_str(),
                        Some("kind") | Some("prop") | Some("meta") | Some("relation") | Some("cu")
                    )
                }) {
                    continue;
                }
            }
            let current_belief = unit.get_number("meta_belief").unwrap_or(0.5);
            // I-005 / S-3: 增量 / ceiling 可配置
            let new_belief = (current_belief
                + (count as f64) * self.thresholds.belief_boost_per_use)
                .min(self.thresholds.belief_ceiling);
            if (new_belief - current_belief).abs() <= f64::EPSILON {
                // 没有变化（已达 ceiling）→ 不写
                continue;
            }
            unit.set(
                "meta_belief",
                Value::Number(
                    serde_json::Number::from_f64(new_belief)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ),
            );
            if let Err(e) = self.store.update(&unit).await {
                crate::plugin_warn!(
                    "agent",
                    "[CognitiveFeedback] flush: failed to update meta_belief for unit '{}': {}",
                    id,
                    e
                );
                continue;
            }
            flushed += 1;
        }
        crate::plugin_info!(
            "agent",
            "[CognitiveFeedback] flush_belief_buffer: {} units written (v25-N2)",
            flushed
        );
        flushed
    }

    /// v25-N3: 进程关闭前的**优雅清理**
    ///
    /// 调用 `flush_belief_buffer()` 把 buffer 中累积的访问计数写回 store，
    /// 避免进程退出时未 flush 的"热单元"访问计数丢失。
    ///
    /// 使用场景：main.rs 收到 SIGINT/SIGTERM、集成测试 teardown。
    /// **设计权衡**：本方法**不**做 Drop 实现（Drop 不能 await），由调用方显式触发。
    ///
    /// 返回：成功 flush 的 unit 数
    pub async fn shutdown(&self) -> usize {
        let n = self.flush_belief_buffer().await;
        crate::plugin_info!(
            "agent",
            "[CognitiveFeedback] shutdown: flushed {} belief updates (v25-N3)",
            n
        );
        n
    }
}
