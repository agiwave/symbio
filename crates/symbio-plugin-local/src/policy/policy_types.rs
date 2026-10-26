use serde::{Deserialize, Serialize};

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
