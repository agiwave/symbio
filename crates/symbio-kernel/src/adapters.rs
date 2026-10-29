//! 时延档位：四层预算的**数据面**（[DE-001 §10](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! 这里只装档位本身（判据是 `budget_ms`，不是「工具」这个名词）；令牌与签发中心
//! （`RuleOnly` / `FullModel` / `TokenIssuer`）是部署侧的依赖注入策略，仍住
//! `symbio_core::adapters`。

/// 时延档位。**判据是时延，不是形式**——约束的是 `budget_ms`，不是"工具"这个名词。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LatencyTier {
    /// 反射：≤80ms，纯规则/查表，**禁模型**（对应人类通路：脊髓反射）。
    Reflex,
    /// 快速：~300ms，只允许**分类**不允许生成（视觉简单反应）。
    Fast,
    /// 深度：秒–分钟，完整规划 + 工具（复杂决策）。
    Deep,
    /// 自主：小时–天，常驻、事件驱动。
    Autonomic,
}

impl LatencyTier {
    /// 四层预算（[DE-001 §8](../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 参数表）。
    pub fn budget_ms(self) -> u64 {
        match self {
            LatencyTier::Reflex => 80,
            LatencyTier::Fast => 300,
            LatencyTier::Deep => 60_000,
            LatencyTier::Autonomic => 86_400_000,
        }
    }

    /// 档位名（入事件的 `tier` 字段——兜底率按层统计的口径锚）。
    pub fn name(self) -> &'static str {
        match self {
            LatencyTier::Reflex => "reflex",
            LatencyTier::Fast => "fast",
            LatencyTier::Deep => "deep",
            LatencyTier::Autonomic => "autonomic",
        }
    }

    /// 从事件载荷的 `tier` 字符串还原档位（未知/缺失 ⇒ `None`——**不静默归类**，
    /// 投影侧把 None 计入可观测的 `unspecified` 桶）。
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "reflex" => Some(LatencyTier::Reflex),
            "fast" => Some(LatencyTier::Fast),
            "deep" => Some(LatencyTier::Deep),
            "autonomic" => Some(LatencyTier::Autonomic),
            _ => None,
        }
    }
}
