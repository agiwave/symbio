//! Mindscape 认知存储引擎
//!
//! 在基础 AgentStore 之上叠加认知能力：
//! - 认知单元验证 + 语义去重
//! - 认知反馈（belief 衰减 + 冲突检测）
//! - 高级搜索（JSON filter + 自动 record_access）

pub(crate) mod cognitive_feedback;
pub(crate) mod scaffold;
pub(crate) mod scaffold_store_impl;

// 测试模块通过 scaffold.rs 末尾的 `#[path = "scaffold_tests.rs"]` 引入，
// 避免与 mod.rs 中的 `mod scaffold_tests;` 重复 include 同一文件。

use crate::plugins::agent::core::{
    AgentConfig, AgentStore, CognitionContext, CognitiveUnit, StorageBackendType,
};
use std::sync::Arc;

use scaffold::MindscapeScaffold;

/// 工厂方法：供 store 注册表使用
///
/// 内部通过注册表创建 base store（由 config.base_backend 决定），
/// 然后自动叠加 Mindscape 认知能力层。
fn create(
    config: &AgentConfig,
    agent_dir: &std::path::Path,
) -> crate::plugins::agent::store::BoxFuture<
    Result<Arc<dyn AgentStore>, crate::plugins::agent::core::StoreError>,
> {
    let config = config.clone();
    let agent_dir = agent_dir.to_path_buf();
    Box::pin(async move {
        // 直接创建 base store（embedding 已集成到 store 内部）
        // P-13: 错误传播，base_backend 注册失败时不再 panic
        let inner =
            crate::plugins::agent::store::create_store(config.base_backend, &config, &agent_dir)
                .await?;
        let ctx = CognitionContext::new(Arc::new(config), agent_dir);
        Ok(Arc::new(MindscapeScaffold::new_with_inner(inner, &ctx).await) as Arc<dyn AgentStore>)
    })
}

// ── 自注册 ──
crate::submit_store_backend!(StorageBackendType::Mindscape, create);

/// 从 seed_cus.jsonl 文件加载核心种子 CU
///
/// seed_cus.jsonl 中 kind 类 CU（如 fact/rule/skill）显式设置 `priority=200`，
/// 表示默认不进入系统提示词候选池（节省 token）。LLM 可自由修改 priority。
///
/// **元数据保护机制**：不需要任何特殊保护。
/// - `init_metacognitive_units` 只在 `Ok(None)` 时插入，已存在的 CU 跳过
/// - 即 LLM 误删了元数据，**重启时自动从 seed_cus.jsonl 重新加载**
pub fn get_default_seed_cus() -> Vec<CognitiveUnit> {
    let content = include_str!("seed_cus.jsonl");
    let mut units = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Ok(value) = serde_json::from_str(line) {
            units.push(value);
        }
    }

    units
}
