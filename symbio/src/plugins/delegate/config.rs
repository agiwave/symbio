//! delegate 的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! 四个字段全部是**判定口径**——没有一条是"机制"：换一组值就换一种委派口味，
//! 关掉开关就回到"主会话直接答"。默认值即出厂行为，字段真源是本结构。

use serde::{Deserialize, Serialize};

/// 委派判定配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegateConfig {
    /// 是否提供三件事实（判定 / 能力目录 / worker 快照）。
    ///
    /// 关掉 = 插件不注册这些事实，系统退化为"主会话直接答"（J2 平凡值），
    /// 不报错。这是"可选插件"的字面实现。
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// **显式前缀**：用户消息以此开头即判「要动手」（用户直接指挥，零歧义）。
    /// 空串 = 不启用前缀触发。
    #[serde(default = "default_force_prefix")]
    pub force_prefix: String,

    /// **关键词命中**即判「要动手」（子串匹配，不区分大小写）。
    /// 空集 = 只认前缀——这是出厂默认：宁可漏判（用户一句话即可补），
    /// 也不要误判（每个闲聊都开 worker 会让状态面噪声化）。
    #[serde(default)]
    pub keywords: Vec<String>,

    /// **主题长度阈值**（字符数）：≥ 阈值即判「要动手」。`0` = 不按长度判定。
    ///
    /// 为什么它与关键词并列而不是二选一：长消息通常是真任务，短寒暄一定不是。
    /// 两者都是**可关**的（0 / 空集），关掉后判据只剩前缀。
    #[serde(default)]
    pub min_chars: usize,

    /// 能力目录最多列几个能力（按名字排序后的条数上限）。
    ///
    /// 目录是给模型看的**摘要**，不是清单本体：超限截断并标注 `…`，
    /// 不静默丢弃（截断可见）。
    #[serde(default = "default_digest_max")]
    pub digest_max: usize,
}

fn default_true() -> bool {
    true
}

/// 默认前缀 `/work `（带尾空格，避免 `/worker`、`/workflow` 这类词被误命中）
fn default_force_prefix() -> String {
    "/work ".to_string()
}

/// 40 条：当前全仓工具量级（20+）留一倍余量
fn default_digest_max() -> usize {
    40
}

impl Default for DelegateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            force_prefix: default_force_prefix(),
            keywords: Vec::new(),
            min_chars: 0,
            digest_max: default_digest_max(),
        }
    }
}

impl DelegateConfig {
    /// 生效的目录条数上限（下界 1，避免配成 0 后目录恒空却看不出原因）
    pub fn effective_digest_max(&self) -> usize {
        self.digest_max.max(1)
    }

    /// 生效的显式前缀（`None` = 未启用）。
    ///
    /// **只判"纯空白"，不 trim 尾空格**——尾空格是**词边界护栏**，不是排版：
    /// 配置 `/work ` 才能让 `/work 干活` 命中而 `/worker 是什么` 不命中（见
    /// [`decide`](super::decide) 的逐字匹配）。若在此处把它 trim 掉，护栏反过来
    /// 变成了误命中源。同理**不** trim 头空格（用户写 `/work` 就是 `/work`）。
    pub fn effective_force_prefix(&self) -> Option<&str> {
        if self.force_prefix.trim().is_empty() {
            None
        } else {
            Some(self.force_prefix.as_str())
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
