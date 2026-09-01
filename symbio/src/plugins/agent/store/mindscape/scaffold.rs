use crate::plugins::agent::core::{now_secs, AgentStore, CognitiveUnit};
use std::collections::HashMap;
use std::sync::Arc as StdArc;

use super::cognitive_feedback::CognitiveFeedback;
use super::get_default_seed_cus;
use crate::plugins::agent::core::CognitionContext;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// 验证快照：Copy-on-Write 模式
#[allow(dead_code)] // 仅在 #[cfg(test)] 中使用
pub(crate) struct VersionedSnapshot {
    pub(crate) data: HashMap<String, CognitiveUnit>,
    pub(crate) created_at: Instant,
    /// 快照是否处于"降级"状态
    ///
    /// `true` = 上次重建时底层 store 查询失败，data 可能是空的或旧的。
    /// 验证器读到 degraded 快照时**应**知道"信任度低"，可：
    /// - 选择"严格"策略：拒绝验证通过（保守）
    /// - 选择"宽松"策略：warn 但放行
    ///
    /// 之前 `build_validation_snapshot` 在 store query 失败时只 `plugin_warn!`
    /// 然后返回空 HashMap，验证器对这种"半残"快照无感知。本次显式标记：
    /// - 调用方（`validate_au`）能根据 `degraded` 决定是否放行
    /// - `persist_with_feedback` 这种"写"路径不会受影响（它直接写 store）
    /// - 监控指标可暴露 degraded 比例，触发告警
    pub(crate) degraded: bool,
}

impl Default for VersionedSnapshot {
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            created_at: Instant::now(),
            // 默认 degraded=true：未经过成功的 build_validation_snapshot 之前，
            // 视为不可信。后续 build 走慢速路径成功后会把 degraded 置回 false。
            degraded: true,
        }
    }
}

pub struct MindscapeScaffold {
    pub(crate) store: Arc<dyn AgentStore>,
    pub(crate) feedback: CognitiveFeedback,
    #[allow(dead_code)] // 仅在 #[cfg(test)] 中使用
    pub(crate) snapshot_cache: RwLock<StdArc<VersionedSnapshot>>,
}

impl MindscapeScaffold {
    /// 以已有的基础 store 构造 MindscapeScaffold
    ///
    /// 由 `store::build_full_store` 调用，实现分层构建。
    /// 与 `new` 的区别：不自行构建基础 store，而是接收已构建好的。
    pub async fn new_with_inner(store: Arc<dyn AgentStore>, context: &CognitionContext) -> Self {
        Self::init_metacognitive_units(&store).await;

        let feedback =
            CognitiveFeedback::new(store.clone()).with_thresholds(context.agent_config.cognition);

        let instance = Self {
            store,
            feedback,
            snapshot_cache: RwLock::new(StdArc::new(VersionedSnapshot {
                data: HashMap::new(),
                created_at: Instant::now(),
                degraded: true,
            })),
        };

        crate::plugin_info!("agent", "MindscapeScaffold initialized (with inner store)");

        instance
    }

    async fn init_metacognitive_units(store: &Arc<dyn AgentStore>) {
        // v9.1：函数名保留以兼容外部调用，语义已重命名为"核心种子 CU"
        // 修复 N+1：使用并发收集 + 单次批量查询
        let mut metacognitive = Vec::new();
        for unit in get_default_seed_cus() {
            let id = unit.id();
            if id.is_empty() {
                continue;
            }
            metacognitive.push((id.to_string(), unit));
        }

        if metacognitive.is_empty() {
            return;
        }

        // 并发查询所有元认知单元的当前状态
        let store_clone = store.clone();
        let query_futures: Vec<_> = metacognitive
            .iter()
            .map(|(id, _)| {
                let store = store_clone.clone();
                let id = id.clone();
                async move {
                    let id_for_call = id.clone();
                    (id, store.get(&id_for_call).await)
                }
            })
            .collect();
        let results = futures::future::join_all(query_futures).await;

        let mut to_insert = Vec::new();
        for ((id, unit), (_, result)) in metacognitive.into_iter().zip(results) {
            match result {
                Ok(None) => {
                    // 元数据 CU 缺失时，注入 created_at/last_access/memory_strength
                    // 元数据 CU 是 schema 定义，不应被遗忘机制清理
                    let now = now_secs();
                    let mut unit = unit;
                    use serde_json::json;
                    if unit.last_access().is_none() {
                        unit.set("_ext_last_access".to_string(), json!(now));
                    }
                    if unit.created_at() == 0 {
                        unit.set("_ext_created_at".to_string(), json!(now));
                    }
                    // memory_strength=86400（约 100 年），让艾宾浩斯曲线对元数据 CU 几乎不衰减
                    if unit.memory_strength() <= 24.0 {
                        unit.set("_ext_memory_strength".to_string(), json!(86400.0_f64));
                    }
                    to_insert.push(unit); // 不存在，需要插入
                }
                Ok(Some(_)) => {} // 已存在，跳过
                // query 错误时 log warn
                Err(e) => {
                    crate::plugin_warn!(
                        "agent",
                        "[Scaffold] metacognitive exists-check failed for {}, treating as absent: {}",
                        id, e
                    );
                    to_insert.push(unit);
                }
            }
            let _ = id; // 抑制未使用变量警告
        }

        if !to_insert.is_empty() {
            if let Err(e) = store.insert_batch(&to_insert).await {
                crate::plugin_warn!("agent", "Failed to persist metacognitive units: {}", e);
            }
        }

        // v9.1 完整性校验：核心关系 prop 与必备 kind 类型 prop 必须存在
        // 若缺漏说明 seed_cus.jsonl 拼错，启动时立即警告（不阻断）
        Self::validate_seed_cus_integrity(store).await;
    }

    /// 校验核心 prop 完整性（启动时）
    ///
    /// 核心关系 prop（is_a / has / causes ...）、必备 kind 类型（fact / skill ...）
    /// 与必备元认知 prop（meta_belief / meta_conflict ...）必须在 `seed_cus.jsonl` 中声明。
    /// 校验失败只记录警告，不阻断启动。
    async fn validate_seed_cus_integrity(store: &Arc<dyn AgentStore>) {
        use crate::plugins::agent::core::{FilterExpr, PageRequest, CORE_RELATION_NAMES};

        // 取 store 中所有 prop CU
        let prop_filter = FilterExpr::is_a("prop");
        let page = PageRequest::first(500);
        let prop_cus = match store.query(&prop_filter, &page).await {
            Ok(r) => r.items,
            Err(e) => {
                crate::plugin_warn!("agent", "prop 完整性校验：查询失败: {}", e);
                return;
            }
        };

        // 数据驱动：从 prop CU 推导实际注册的关系名
        let actual_relations: std::collections::HashSet<String> = prop_cus
            .iter()
            .filter(|p| p.is_relation_prop())
            .map(|p| p.id().to_string())
            .collect();
        let prop_ids: std::collections::HashSet<String> = prop_cus
            .iter()
            .map(|p| p.id().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        // kind 树必备类型（meta 不在此处：meta 是 prop 子类，与 relation 平行）
        let expected_kinds = ["fact", "skill", "rule"];
        // meta 命名空间下的必备横切属性 prop
        let expected_meta_props = [
            "meta_belief",
            "meta_conflict",
            "meta_learning",
            "meta_reflection",
            "meta_adaptation",
        ];

        let mut missing: Vec<String> = Vec::new();
        for name in CORE_RELATION_NAMES {
            if !actual_relations.contains(*name) {
                missing.push(format!("relation={}", name));
            }
        }
        for k in expected_kinds.iter() {
            if !prop_ids.contains(*k) {
                missing.push(format!("kind={}", k));
            }
        }
        for m in expected_meta_props.iter() {
            if !prop_ids.contains(*m) {
                missing.push(format!("meta_prop={}", m));
            }
        }
        if missing.is_empty() {
            crate::plugin_info!(
                "agent",
                "prop 完整性校验通过（{} 个关系 + {} 个核心 kind + 5 个元认知 prop）",
                actual_relations.len(),
                expected_kinds.len()
            );
        } else {
            crate::plugin_warn!(
                "agent",
                "prop 完整性校验发现缺失：{:?}。请检查 seed_cus.jsonl 是否完整。",
                missing
            );
        }
    }

    /// 使快照缓存失效（仅测试使用）
    #[allow(dead_code)]
    pub(crate) async fn invalidate_snapshot_cache(&self) {
        let was_degraded = self.snapshot_cache.read().await.degraded;
        let mut cache = self.snapshot_cache.write().await;
        *cache = StdArc::new(VersionedSnapshot {
            data: HashMap::new(),
            created_at: Instant::now() - std::time::Duration::from_secs(60),
            degraded: was_degraded,
        });
    }
}

/// Drop 兜底
///
/// **正常流程**：消费者应在收到 SIGINT / SIGTERM / 优雅退出时调用
/// `shutdown().await` 显式 flush belief buffer + 取消后台 rebuild。
///
/// **兜底流程**：消费者忘调 `shutdown()` 时，Drop 通过 `Handle::spawn`
/// 异步执行一次 `flush_belief_buffer()`，把累积的访问计数尽量写回 store
///（cancel_background_tasks 同步调用，无 await）。
///
/// 行为矩阵：
/// | 当前上下文              | Drop 行为                                    |
/// |-------------------------|---------------------------------------------|
/// | 同步代码（无 runtime）  | 仅 cancel_background_tasks，log warning    |
/// | tokio runtime 上下文    | cancel_background_tasks + spawn async flush|
/// | 已是 tokio worker 线程  | cancel_background_tasks + spawn async flush|
impl Drop for MindscapeScaffold {
    fn drop(&mut self) {
        // 1) 同步部分：cancel_background_tasks（无 await，安全）
        self.store.cancel_background_tasks();

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let feedback = self.feedback.clone();
                handle.spawn(async move {
                    let n = feedback.pending_belief_updates().await;
                    if n > 0 {
                        let flushed = feedback.flush_belief_buffer().await;
                        crate::plugin_info!(
                            "agent",
                            "[MindscapeScaffold] Drop: best-effort flush wrote {} belief updates",
                            flushed
                        );
                    }
                });
            }
            Err(_) => {
                crate::plugin_warn!(
                    "agent",
                    "[MindscapeScaffold] Drop: no tokio runtime available, \
                     skipped belief_buffer flush. For data safety, prefer explicit shutdown hookup."
                );
            }
        }
    }
}

// 测试代码位于 `scaffold_tests.rs`（同目录 sibling 文件，体积过大故外置）

#[cfg(test)]
#[path = "scaffold_tests.rs"]
mod tests;
