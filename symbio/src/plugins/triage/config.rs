//! Triage 插件自己的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 为什么配置归本插件，而不是 `session` 的配置面
//!
//! 「用不用规则短路」是**本插件内部策略**（有没有那张表、要不要走它）。
//! 把它塞进 `SessionConfig`，等于让调用方知道「判决插件内部有一张规则表」——
//! 调用方于是要为一段它看不见的策略维护一个开关，而插件换个策略就要改调用方的配置。
//!
//! 判据在 `docs/plan/06-会话响应性落地.md` §4.2 的对照表里已经写着：
//! **「独立开关 / 独立模型」的落地形态是「插件目录 = 配置目录（各自 `PLUGIN.yml`）」**。
//! 本文件是这句话的第一个实例。
//!
//! `session` 侧只留一个开关：`triage_enabled`（要不要请判决）——那才是调用方的事。

use serde::{Deserialize, Serialize};

/// Triage 配置 —— 本插件的旋钮（字段真源）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageConfig {
    /// 规则短路开关（**J2 平凡值：`false`**）。
    ///
    /// - `true`（默认）：问候 / 致谢 / 确认 / 空输入由规则表直接判决，
    ///   **零 LLM 往返**；
    /// - `false`：规则表不生效，全部输入落快速档（慢但正确——功能完整，
    ///   只是每次多付一次分类往返）。
    ///
    /// 平凡值不是「预留开关」：它可被 e2e 直接验证（关掉它，同一句话从
    /// 「零 LLM 请求」变成「一次分类请求」），因此它是一条**参数**，不是一段预留。
    #[serde(default = "default_rule_shortcut")]
    pub rule_shortcut: bool,
}

pub fn default_rule_shortcut() -> bool {
    true
}

impl Default for TriageConfig {
    fn default() -> Self {
        Self {
            rule_shortcut: default_rule_shortcut(),
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
