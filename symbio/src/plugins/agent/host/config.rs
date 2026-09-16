//! agent 插件的配置（`<homedir>/plugins/agent/PLUGIN.yml`）。
//!
//! 四个字段 = 两组容量闸门，与 work 插件同一套口径（**写侧拒绝、读侧截断**）：
//!
//! | 字段 | 闸门 | 位置 | 超限行为 |
//! |---|---|---|---|
//! | `item_max_bytes` | 人格条目写入 | [`BundleStore::write_item`](super::store::BundleStore::write_item) | **拒绝** |
//! | `identity_inject_max_bytes` | 人格注入 | [`identity_segment`](super::prompt::identity_segment) | **截断** + 告知地址 |
//! | `memory_max_bytes` | 智能体记忆写入 | [`MemoryFile::write`](crate::symbio_core::MemoryFile::write) | **拒绝** |
//! | `memory_inject_max_bytes` | 智能体记忆注入 | [`MemoryFile::inject`](crate::symbio_core::MemoryFile::inject) | **截断** + 告知地址 |
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
    /// 单个 bundle 条目文件（提示词 / 技能 / MCP 配置）的**写入**字节上限。
    #[serde(default = "default_item_max_bytes")]
    pub item_max_bytes: usize,

    /// 人格注入系统提示词的**字节**上限（超出部分截断，靠地址读取全文）。
    #[serde(default = "default_identity_inject_max_bytes")]
    pub identity_inject_max_bytes: usize,

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

/// 8 KiB：约两千余汉字，够放人格主体，又不至于把每轮上下文吃掉
fn default_identity_inject_max_bytes() -> usize {
    8 * 1024
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
            identity_inject_max_bytes: default_identity_inject_max_bytes(),
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

    /// 生效的人格注入上限（下界 1 字节，理由同上）
    pub fn effective_inject_bytes(&self) -> usize {
        self.identity_inject_max_bytes.max(1)
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
