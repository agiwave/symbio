//! `calibration` 投影（S9 第 22 步，[roadmap/S11 §3/§5](../../../../docs/003-design/041-技能编译与自我改进.md "DE-041-技能编译与自我改进")）。
//!
//! 技能会持续犯错而自己不知道——架构没有"技能自动失效"机制（S11 §7 诚实划界），
//! 只能靠**校准观测**后由路由者回退：每个技能记录使用次数与回退次数，
//! 置信度 = 1 − 回退率。低于阈值 ⇒ [`crate::symbio_core::actors::SkillRouter`]
//! 不得走技能路径（反自动化回退）。
//!
//! 观测面：校准断言落**自己的格子** `memory.calibrated`（`payload: { skill_id,
//! fallback }`）。它曾复用 `memory.recalled`（S11 §3「不加新格子」），代价是读侧
//! 只能靠载荷字段判别自己的输入——同一张格子两种语义，任何将来带这两个字段的格子
//! 都会被算进置信度，而当时唯一的防线是「编译事件恰好不带 `fallback`」。

use super::super::event::Event;
use super::Projection;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一个技能的校准统计。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SkillStats {
    /// 技能 id（编译事件的 `skill_id`）。
    pub skill_id: String,
    /// 使用次数（命中 + 回退都算一次使用）。
    pub uses: u64,
    /// 其中回退到完整推理的次数。
    pub fallbacks: u64,
}

impl SkillStats {
    /// 置信度 = 1 − 回退率（无使用 = 无数据，按 1.0 处理——不给新技能判死刑）。
    pub fn confidence(&self) -> f64 {
        if self.uses == 0 {
            1.0
        } else {
            1.0 - (self.fallbacks as f64 / self.uses as f64)
        }
    }
}

/// 校准视图：按 skill_id 字典序（BTreeMap ⇒ 逐字节确定）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CalibrationView {
    pub by_skill: BTreeMap<String, SkillStats>,
}

impl CalibrationView {
    /// 某技能的统计（未使用的技能 ⇒ 零数据）。
    pub fn of(&self, skill_id: &str) -> SkillStats {
        self.by_skill.get(skill_id).cloned().unwrap_or(SkillStats {
            skill_id: skill_id.to_string(),
            uses: 0,
            fallbacks: 0,
        })
    }
}

/// `calibration` 投影：从事件切片统计各技能的置信度。
pub fn calibration() -> Projection<CalibrationView> {
    Projection::new(|events: &[Event], now| {
        let mut by_skill: BTreeMap<String, SkillStats> = BTreeMap::new();
        for e in events {
            if e.ts > now {
                continue;
            }
            if e.kind != crate::symbio_core::event::EVENT_MEMORY_CALIBRATED {
                continue;
            }
            let Some(skill_id) = e.payload.get("skill_id").and_then(|v| v.as_str()) else {
                continue;
            };
            // 按**格子**认自己的输入，不按载荷字段猜——`memory.calibrated` 是校准观测的专属
            // 格子。判别回到闭集之后，「别的格子恰好不带这两个字段」不再是读侧的前提。
            // 格子对了字段仍逐个校：缺字段的事件**不参与统计**，而不是被当成「未回退」
            // ——后者会让 uses 涨而 fallbacks 不涨，凭空造出高置信度。
            let Some(fallback) = e.payload.get("fallback").and_then(|v| v.as_bool()) else {
                continue;
            };
            let st = by_skill
                .entry(skill_id.to_string())
                .or_insert_with(|| SkillStats {
                    skill_id: skill_id.to_string(),
                    uses: 0,
                    fallbacks: 0,
                });
            st.uses += 1;
            if fallback {
                st.fallbacks += 1;
            }
        }
        CalibrationView { by_skill }
    })
}

#[cfg(test)]
#[path = "calibration.test.rs"]
mod tests;
