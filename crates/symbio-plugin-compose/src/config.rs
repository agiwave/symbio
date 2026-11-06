//! Compose 插件自己的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 为什么本插件现在有配置了（此前没有）
//!
//! 模块文档写着「**不读配置**：本插件没有自己的配置面」——那时它确实没有
//! 可配的东西：措辞没有比"能生成就生成"更值得开关的分支。但有一件**真的有分歧**
//! 的事：**指令段怎么写**——它直接决定答话像不像"这个助手本人"，而调它不该需要
//! 改代码重编译。
//!
//! 「要不要用 `compose`」仍然**不在这里**——那归调用方（会话侧的措辞策略）。
//! 判据同样是：插件目录 = 配置目录，各管各的旋钮。
//!
//! ## 键的平凡值
//!
//! | 键 | 平凡值 | 平凡值下 |
//! |---|---|---|
//! | `instruction` | 缺席 | 用内置的那份指令段 |
//!
//! 「用哪个模型」不再是本插件的旋钮：生成已搬到 `model.assist` 口债，执行体
//! 归 model 端口，选哪个 provider 因此是**会话**的选择（与 compact 同一条路）。

use serde::{Deserialize, Serialize};

/// Compose 配置 —— 本插件的旋钮（字段真源）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeConfig {
    /// 生成指令段的覆盖（缺席 ⇒ 用内置那份，见 `wording::INSTRUCTION`）。
    ///
    /// 覆盖的是**指令段**，不是整个系统提示词：注册段（人格 / 记忆）必须照旧拼在
    /// 前面——同一个人在两处口吻不同，比一句措辞不理想糟得多。
    #[serde(default)]
    pub instruction: Option<String>,
}

impl ComposeConfig {
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
