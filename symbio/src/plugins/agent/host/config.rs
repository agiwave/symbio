//! agent 插件的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! 三个字段，与 work 插件同一套口径（**写侧拒绝、读侧截断**）：
//!
//! | 字段 | 闸门 | 位置 | 超限行为 |
//! |---|---|---|---|
//! | `item_max_bytes` | Agent 目录内文件写入 | [`BundleStore::write_item`](super::store::BundleStore::write_item) | **拒绝** |
//! | `memory_max_bytes` | 智能体记忆写入 | [`MemoryFile::write`](crate::symbio_core::MemoryFile::write) | **拒绝** |
//! | `memory_inject_max_bytes` | 智能体记忆注入 | [`MemoryFile::inject`](crate::symbio_core::MemoryFile::inject) | **截断** + 告知地址 |
//!
//! ⚠️ 人格不再有注入闸门：v2 的人格就在 `AGENTS.md` 里（§6），由该 Agent 的
//! `work` 插件实例按它自己的记忆闸门注入——本插件不再另算一份预算，否则同一份
//! 内容会被两个所有者各截一次。
//!
//! 为什么写侧是拒绝：人格与记忆都是**跨会话生效**的东西，写入被截断意味着
//! 模型以为改好了、实际少了一块——这种失败没有任何报错，只能靠「拒绝」
//! 把它变成一次显式的、可重试的失败。
//!
//! ⚠️ 后两行**不落在本插件**：智能体记忆的读写与两道闸门都在内核
//! （`symbio_core::memory`），与 work / session 两层共用同一份实现。本插件只提供
//! 落位与地址，配置在这里的作用是**把闸门取值喂给内核**。

use serde::{Deserialize, Serialize};

/// agent 插件配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent 目录内单个文件的**写入**字节上限。
    #[serde(default = "default_item_max_bytes")]
    pub item_max_bytes: usize,

    /// 智能体记忆（bundle 根下的 `AGENTS.md`）的**写入**字节上限。
    #[serde(default = "default_memory_max_bytes")]
    pub memory_max_bytes: usize,

    /// 智能体记忆注入系统提示词的**字节**上限。
    #[serde(default = "default_memory_inject_max_bytes")]
    pub memory_inject_max_bytes: usize,
}

/// 32 KiB：一个人格片段写到这里已经不是「片段」了
fn default_item_max_bytes() -> usize {
    32 * 1024
}

/// 16 KiB：与工作区记忆同一口径（两者是同一类东西，只是作用域不同）
fn default_memory_max_bytes() -> usize {
    16 * 1024
}

/// 4 KiB：与工作区记忆同一口径
fn default_memory_inject_max_bytes() -> usize {
    4 * 1024
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            item_max_bytes: default_item_max_bytes(),
            memory_max_bytes: default_memory_max_bytes(),
            memory_inject_max_bytes: default_memory_inject_max_bytes(),
        }
    }
}

impl AgentConfig {
    /// 生效的条目写入上限（下界 1 字节，避免配成 0 后一切写入都失败却看不出原因）
    pub fn effective_item_max_bytes(&self) -> usize {
        self.item_max_bytes.max(1)
    }

    /// 生效的记忆写入上限
    pub fn effective_memory_max_bytes(&self) -> usize {
        self.memory_max_bytes.max(1)
    }

    /// 生效的记忆注入上限
    pub fn effective_memory_inject_bytes(&self) -> usize {
        self.memory_inject_max_bytes.max(1)
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
