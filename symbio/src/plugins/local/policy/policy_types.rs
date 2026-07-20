use serde::{Deserialize, Serialize};

/// 工具风险等级
///
/// 派生 `PartialOrd`/`Ord`：声明顺序即等级顺序（Low < Medium < High），
/// 供「执行风险等级」阈值比较使用（`tool 风险 > 阈值` ⇒ 需审批）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// 低风险：只读操作，安全无害
    Low,
    /// 中风险：可能修改状态，但影响有限
    #[default]
    Medium,
    /// 高风险：可能产生破坏性影响或访问敏感资源
    High,
}

/// Agent 自主级别
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    /// 只读：只能观察，不能操作
    ReadOnly,
    /// 监督：可以操作，但危险操作需要批准
    #[default]
    Supervised,
    /// 完全自主：在策略范围内自主执行
    Full,
}
