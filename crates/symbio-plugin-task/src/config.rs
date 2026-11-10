//! Task 能力包自己的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! 就绪集容量此前由 `SessionConfig::task_cap` 代管，轮首投影成 `readyset_cap`
//! 这一格给本包读（D-252）。D-427 把真源收回能力包：本插件挂载即贡献
//! `readyset_cap`，未挂载 ⇒ 那一格缺席 ⇒ 读方按 `TASK_CAP` 出厂值收敛。
//! 键名沿用读方一直在读的 `readyset_cap`，「谁写」换了、「读什么」不换。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// 就绪集容量（出厂 = `symbio_plugin_api::TASK_CAP`，与迁移前一致）。
    #[serde(default = "default_readyset_cap")]
    pub readyset_cap: usize,
}

pub fn default_readyset_cap() -> usize {
    symbio_plugin_api::TASK_CAP
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            readyset_cap: default_readyset_cap(),
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
