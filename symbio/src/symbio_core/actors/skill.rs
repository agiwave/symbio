//! 技能编译与路由（[roadmap/S11](../../../../docs/007-task/131-技能编译与自我改进.md ")）：成功轨迹固化为技能事实、按置信度决定快路 / 回退。

use crate::symbio_core::Event;

/// 技能的认知内容标签（`memory.encoded` 载荷里的 `tag`）。
///
/// 名字是数据、单点定义（D-043）：写方（[`SkillCompiler::compile`]）、读侧路由
/// （`plugins/loop/skill_events.rs`）与读数口（`plugins/session/stats.rs`）都 `use`
/// 这一处。两处各写一个字面量时，改一边不改另一边 ⇒ 技能被当成普通记忆召回却不进
/// 路由，全程没有任何报错。
pub const SKILL_TAG: &str = "skill";

/// 技能编译者（S9 第 22 步，[roadmap/S11](../../../../docs/007-task/131-技能编译与自我改进.md ")）。
///
/// 编译 = 把成功执行轨迹固化为一条 `memory.encoded{tag:"skill"}` 事件——
/// **技能是事实，不是特殊类型**。编译产出的技能事件**必带溯源**
/// （`produced_by` 指向源轨迹；I2 断言，技能溯源 100%）。
pub struct SkillCompiler;

impl SkillCompiler {
    /// 编译一条技能。`source_seq`：源轨迹（成功任务的终态事件）seq。
    ///
    /// 信封里的 `actor` / `turn` 是**占位**（本机主智能体 / `0`）：核心契约只管
    /// 「这条事件说什么」，记在谁头上、发生在哪一轮由部署的写方落格前归位
    /// （`plugins/session/skill_events.rs::compile`——`memory × *` 的 actor 是属主，
    /// 召回投影按它过滤，占位不归位技能就进不了召回视图）。
    pub fn compile(&self, skill_id: &str, trigger: &str, response: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("skill-{skill_id}-{source_seq}"),
            crate::symbio_core::event::EVENT_MEMORY_ENCODED,
            crate::symbio_core::event::Entity::Memory,
            crate::symbio_core::event::Verb::Opened,
            0,
            // 占位 actor 一律走常量，不写字面量：`session/xxx` 形态的裸串会被
            // plugin-entry-audit 的 E-005 当成路由地址查一遍（D-043：名字是数据、
            // 单点定义）。写方落格前归位，见 `skill_events::compile`。
            crate::symbio_core::authz::PRINCIPAL_MAIN,
        )
        .with_produced_by(source_seq) // 溯源 100%：指向源轨迹
        .with_payload(serde_json::json!({
            "content": response,
            "tag": SKILL_TAG,
            "skill_id": skill_id,
            "trigger": trigger,
        }))
    }
}

/// 路由结论：命中技能走快路（`budget_ms = 80`），否则**必须回退**完整推理
/// （反自动化回退，Beilock & Carr 2001——全自动化在压力下以异常方式失效）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillRoute {
    /// 技能命中：反射档快路。
    SkillFastPath { budget_ms: u64 },
    /// 回退完整推理：快速档。
    ReasonerFallback { budget_ms: u64 },
}

/// 技能路由者：按校准置信度决定走技能还是回退（校准值低于阈值 ⇒ 不得走技能路径）。
pub struct SkillRouter;

impl SkillRouter {
    pub fn route(&self, confidence: f64, threshold: f64) -> SkillRoute {
        if confidence >= threshold {
            SkillRoute::SkillFastPath {
                budget_ms: crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms(),
            }
        } else {
            SkillRoute::ReasonerFallback {
                budget_ms: crate::symbio_core::adapters::LatencyTier::Fast.budget_ms(),
            }
        }
    }
}
