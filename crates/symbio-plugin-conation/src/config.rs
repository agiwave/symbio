//! Conation 能力包自己的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! 「要不要让『欲』升格」此前由 `SessionConfig::conation_enabled` 代管，并投影成
//! `session.config` 的一格给本包按共享名读（D-379）。D-427 把真源收回能力包：
//! 本插件挂载即在开轮收集期贡献 `conation_enabled`，未挂载 ⇒ 那一格缺席 ⇒
//! 读方按自己的出厂值 `false` 收敛，宿主不需要认识本能力。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConationConfig {
    /// 自主层升格开关（出厂 `false`，与迁移前 `SessionConfig` 一致）。
    #[serde(default = "default_conation_enabled")]
    pub conation_enabled: bool,
}

pub fn default_conation_enabled() -> bool {
    false
}

impl Default for ConationConfig {
    fn default() -> Self {
        Self {
            conation_enabled: default_conation_enabled(),
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
