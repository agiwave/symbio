//! Compose 插件自己的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 为什么本插件现在有配置了（S3 时没有）
//!
//! S3 的模块文档写着「**不读配置**：本插件没有自己的配置面」——那时它确实没有
//! 可配的东西：措辞没有比"能生成就生成"更值得开关的分支。S6 带来了两件**真的有
//! 分歧**的事：
//!
//! - **用哪个模型**：答话是唯一一段由模型自由组织、用户会逐字读的文本，
//!   值得用比判决更好的模型；
//! - **指令段怎么写**：它直接决定答话像不像"这个助手本人"，而调它不该需要改代码
//!   重编译。
//!
//! 「要不要用 `compose`」仍然**不在这里**——那归调用方（`SessionConfig::reply_enabled`）。
//! 判据见 `docs/plan/06-会话响应性落地.md` §4.2：插件目录 = 配置目录，各管各的旋钮。
//!
//! ## 两个键各自的平凡值
//!
//! | 键 | 平凡值 | 平凡值下 |
//! |---|---|---|
//! | `model` | 缺席 | 用**会话选定的**模型 |
//! | `instruction` | 缺席 | 用内置的那份指令段 |

use serde::{Deserialize, Serialize};

/// Compose 配置 —— 本插件的旋钮（字段真源）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeConfig {
    /// 措辞用的 **provider id**（`<根>/model/<id>`）。
    ///
    /// 缺席 / 空串 ⇒ 用会话选定的那一个（平凡值）。配置的 id 取不到时同样落回它
    /// ——**降级而不失效**：一个 typo 不该让答话整条产线消失（见
    /// `CapabilityVisitor::resolve_model_provider`）。
    ///
    /// **温度**不必在这里再写一个字段：它是 provider 条目自己的参数
    /// （`<根>/model/<id>/provider.json`），换一个条目就是换温度。
    #[serde(default)]
    pub model: Option<String>,

    /// 生成指令段的覆盖（缺席 ⇒ 用内置那份，见 `compose::INSTRUCTION`）。
    ///
    /// 覆盖的是**指令段**，不是整个系统提示词：注册段（人格 / 记忆）必须照旧拼在
    /// 前面——同一个人在两处口吻不同，比一句措辞不理想糟得多。
    #[serde(default)]
    pub instruction: Option<String>,
}

impl ComposeConfig {
    /// 生效的模型 id。**空串与缺席同义**——设置页把输入框清空得到的是 `""`，
    /// 不是"删掉这个键"。
    pub fn model_id(&self) -> Option<&str> {
        non_empty(self.model.as_deref())
    }

    /// 生效的指令段覆盖（`None` ⇒ 调用方用内置常量）
    pub fn instruction_override(&self) -> Option<&str> {
        non_empty(self.instruction.as_deref())
    }
}

/// 空串 / 纯空白 ⇒ `None`（"没填"的两种写法同义）
fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
