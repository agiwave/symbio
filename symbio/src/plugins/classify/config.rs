//! Classify 插件自己的配置（`<本插件目录>/PLUGIN.yml`）。
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
//! `session` 侧只留一个开关：`classify_enabled`（要不要请判决）——那才是调用方的事。
//!
//! ## 三个键各自的平凡值
//!
//! | 键 | 平凡值 | 平凡值下 |
//! |---|---|---|
//! | `rule_shortcut` | `false` | 规则表不生效，全部落快速档（慢但正确） |
//! | `model` | 缺席 | 用**会话选定的**模型（= 没有"独立模型"这件事之前的行为） |
//! | `system_prompt` | 缺席 | 用内置的那份分类提示词 |
//!
//! 三个平凡值都能被直接验证，因此都是**参数**、不是预留（J2）。

use serde::{Deserialize, Serialize};

/// Classify 配置 —— 本插件的旋钮（字段真源）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyConfig {
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

    /// 判决用的 **provider id**（`<根>/model/<id>`）。
    ///
    /// 判决是一次四选一分类——**最便宜最快**的模型就够，没必要用会话那个大模型。
    /// 这正是「一个能力一个插件」的直接回报：本插件的模型与会话的模型互不牵制，
    /// 而**温度**不必在这里再写一个字段——它是 provider 条目自己的参数
    /// （`<根>/model/<id>/provider.json`），换一个条目就是换温度。
    ///
    /// 缺席 / 空串 ⇒ 用会话选定的那一个（平凡值）。配置的 id 取不到时同样落回它
    /// ——**降级而不失效**：一个 typo 不该让判决整条产线消失（见
    /// `CapabilityVisitor::resolve_model_provider`）。
    #[serde(default)]
    pub model: Option<String>,

    /// 分类器系统提示词的覆盖（缺席 ⇒ 用内置那份，见 `classify::SYSTEM_PROMPT`）。
    ///
    /// 可覆盖是为了**可 A/B**：提示词的措辞直接决定四选一的准确率，而调它不该
    /// 需要改代码重编译。四条约束不能丢（只输出一个词 / 四个词的词表 / 只读最后一
    /// 句话 / 拿不准走 `work`）——词表本身在 `classify::CHOICES` 里，**改提示词
    /// 不会改行为映射**。
    #[serde(default)]
    pub system_prompt: Option<String>,
}

pub fn default_rule_shortcut() -> bool {
    true
}

impl Default for ClassifyConfig {
    fn default() -> Self {
        Self {
            rule_shortcut: default_rule_shortcut(),
            model: None,
            system_prompt: None,
        }
    }
}

impl ClassifyConfig {
    /// 生效的模型 id。**空串与缺席同义**——设置页把输入框清空得到的是 `""`，
    /// 不是"删掉这个键"；把它当成一个 id 去查，只会查不到。
    pub fn model_id(&self) -> Option<&str> {
        non_empty(self.model.as_deref())
    }

    /// 生效的提示词覆盖（`None` ⇒ 调用方用内置常量）
    pub fn prompt_override(&self) -> Option<&str> {
        non_empty(self.system_prompt.as_deref())
    }
}

/// 空串 / 纯空白 ⇒ `None`（"没填"的两种写法同义）
fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
