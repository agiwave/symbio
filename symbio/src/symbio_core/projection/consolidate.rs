//! `consolidate` —— 巩固的接受边界（S5 第 13 步，[plan/01 §9.3](../../../../docs/plan/01-核心架构.md)、
//! [plan/05 §4](../../../../docs/plan/05-模块架构.md) ③ 加 consolidate）。
//!
//! ## 架构只提供位置，不提供算法
//!
//! 巩固策略（压什么、留什么）是**算法问题**（[roadmap/S06 §7](../../../../docs/plan/roadmap/S06-长期记忆与语义检索.md)）；
//! 本模块只固化**接受条件**——它是两条，不是一条（01 §9.3）：
//!
//! | 约束 | 参数 | 性质 | 拒收行为 |
//! |---|---|---|---|
//! | 形态 | `max_gen = 3` | 代数上界（防无限推进，G7） | [`Rejection::RejectedMaxGen`] |
//! | 质量 | `min_fidelity = 0.7` | 保真度下界（防失真固化，G7 质量侧） | [`Rejection::RejectedLowFidelity`] |
//!
//! 只写代数上界时，系统会产出「代数合规但已经失真」的记忆——这就是 N8
//! （保真度 100%）要堵的静默失效：失真的巩固一旦入库，后续决策全被污染。

/// 巩固参数（[plan/01 §8](../../../../docs/plan/01-核心架构.md) `projection.param` 的两个取值）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsolidateParams {
    /// 巩固代数上界（派生代数 ≤ max_gen，防代数发散）。
    pub max_gen: u32,
    /// 保真度下界（0.0–1.0；低于下界的巩固不得入库）。
    pub min_fidelity: f64,
}

impl Default for ConsolidateParams {
    fn default() -> Self {
        // 权威取值（01 §8）：max_gen = 3、min_fidelity = 0.7。
        ConsolidateParams {
            max_gen: 3,
            min_fidelity: 0.7,
        }
    }
}

/// 拒收形态（01 §9.3 表的两臂）。
#[derive(Debug, Clone, PartialEq)]
pub enum Rejection {
    /// 代数超界：派生代数超过 `max_gen`。
    RejectedMaxGen { generation: u32, max: u32 },
    /// 保真度不足：低于 `min_fidelity`——拒收，**不是降标入库**。
    RejectedLowFidelity { fidelity: f64, min: f64 },
}

/// 巩固接受判定：通过 ⇒ 该 `memory.consolidated` 事件可入库（带 generation /
/// fidelity 载荷）；否则拒收。**拒收不是错误**——巩固者换策略重试或放弃，
/// 但失真记忆永远进不了 Log。
pub fn accept(params: &ConsolidateParams, generation: u32, fidelity: f64) -> Result<(), Rejection> {
    if generation > params.max_gen {
        return Err(Rejection::RejectedMaxGen {
            generation,
            max: params.max_gen,
        });
    }
    if fidelity < params.min_fidelity {
        return Err(Rejection::RejectedLowFidelity {
            fidelity,
            min: params.min_fidelity,
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "consolidate.test.rs"]
mod tests;
