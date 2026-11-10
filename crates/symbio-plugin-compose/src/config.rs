//! Compose 插件自己的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 为什么本插件现在有配置了（此前没有）
//!
//! 模块文档写着「**不读配置**：本插件没有自己的配置面」——那时它确实没有
//! 可配的东西：措辞没有比"能生成就生成"更值得开关的分支。但有一件**真的有分歧**
//! 的事：**指令段怎么写**——它直接决定答话像不像"这个助手本人"，而调它不该需要
//! 改代码重编译。
//!
//! 「要不要用 `compose`」现在也在**这里**（D-427）：`compose_enabled` 曾由
//! `SessionConfig` 代管，并且会话还要去债表按 `PLUGIN_ID_COMPOSE` 查本插件在场，
//! 才能算出有效开关。如今本插件挂载即贡献自己的开关值，未挂载 ⇒ 那一格缺席 ⇒
//! 读方按出厂值 `false` 收敛；会话不再认识本插件的名字。判据同样是：插件目录 =
//! 配置目录，各管各的旋钮。
//!
//! ## 键的平凡值
//!
//! | 键 | 平凡值 | 平凡值下 |
//! |---|---|---|
//! | `instruction` | 缺席 | 用内置的那份指令段 |
//! | `model` | 缺席 | 用**会话选定的** provider（= 没有"偏好"之前的行为） |
//!
//! 生成已搬到 `model.assist` 口债，执行体归 model 端口，本插件仍**不直连
//! provider**。但「用哪个档」是能力方的知识：措辞知道要写得好，就在请求里带上
//! 偏好；会话选定值是**缺省**，不是天花板（D-348）。

use serde::{Deserialize, Serialize};

/// Compose 配置 —— 本插件的旋钮（字段真源）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeConfig {
    /// 要不要生成对话面措辞（D-427：本插件自持的开关，出厂 `true`）。
    ///
    /// 出厂值必须与迁移前 `SessionConfig::compose_enabled` 一致；过去会话还要
    /// 「配置值 ∧ 债表里 compose 在场」才算有效，现在**挂载即贡献**：卸载时这一格
    /// 直接缺席，读方按 `false` 收敛，等价且不需要按名字查表。
    #[serde(default = "default_compose_enabled")]
    pub compose_enabled: bool,

    /// 生成指令段的覆盖（缺席 ⇒ 用内置那份，见 `wording::INSTRUCTION`）。
    ///
    /// 覆盖的是**指令段**，不是整个系统提示词：注册段（人格 / 记忆）必须照旧拼在
    /// 前面——同一个人在两处口吻不同，比一句措辞不理想糟得多。
    #[serde(default)]
    pub instruction: Option<String>,

    /// 措辞用的**偏好 provider id**（`<根>/model/<id>`）。
    ///
    /// 措辞是一次「写得好」的生成，本插件知道该用哪个档，就在 assist 请求里带上
    /// 这个偏好——与 `classify.model` 同一形态（D-348：请求偏好 > 会话选定 >
    /// 端口缺省）。缺席 / 空串 / 取不到该条目 ⇒ 回落**会话选定**的那一个，
    /// 降级而不失效。
    #[serde(default)]
    pub model: Option<String>,
}

pub fn default_compose_enabled() -> bool {
    true
}

impl Default for ComposeConfig {
    fn default() -> Self {
        Self {
            compose_enabled: default_compose_enabled(),
            instruction: None,
            model: None,
        }
    }
}

impl ComposeConfig {
    /// 生效的指令段覆盖（`None` ⇒ 调用方用内置常量）
    pub fn instruction_override(&self) -> Option<&str> {
        non_empty(self.instruction.as_deref())
    }

    /// 生效的偏好 provider id。空串与缺席同义（设置页清空得到的是 `""`）。
    pub fn model_id(&self) -> Option<&str> {
        non_empty(self.model.as_deref())
    }
}

/// 空串 / 纯空白 ⇒ `None`（"没填"的两种写法同义）
fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
