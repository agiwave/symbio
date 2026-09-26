//! agent 插件的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! 两个字段，与 work / session 几层同一套口径（**写侧拒绝、读侧截断**）：
//!
//! | 字段 | 闸门 | 位置 | 超限行为 |
//! |---|---|---|---|
//! | `memory_max_bytes` | 智能体自身的 `AGENTS.md` 写入 | [`MemoryFile::write`](crate::providers::MemoryFile::write) | **拒绝** |
//! | `memory_inject_max_bytes` | 智能体自身的 `AGENTS.md` 注入 | [`MemoryFile::inject`](crate::providers::MemoryFile::inject) | **截断** + 告知地址 |
//!
//! 「智能体自身的 `AGENTS.md`」有**两个作用域**：系统态 `{homedir}/AGENTS.md`
//! （对所有会话生效，用户可在设置页编辑）与子智能体态 `<agentdir>/AGENTS.md`
//! （随 agent 目录分发，选中该智能体时生效）——见 [`super::instruction`] /
//! [`super::memory`]。两者是同一类东西（同一种文件、同一套读写与注入语义），
//! 只是作用域不同，因此**共用一对闸门**；真要分开配时再拆字段，不预先发明这个区分。
//!
//! 注入为什么比写入小得多：注入的正文**每一轮**都在烧上下文（4 KiB），
//! 写入是一次性的（16 KiB，超出部分模型按地址 `vdfs_read` 取全文）。
//!
//! 为什么写侧是拒绝：指令与记忆都是**跨会话生效**的东西，写入被截断意味着
//! 模型以为改好了、实际少了一块——这种失败没有任何报错，只能靠「拒绝」
//! 把它变成一次显式的、可重试的失败。
//!
//! ⚠️ 读写与闸门本身**不落在本插件**：都在共享实现（`providers/memory`），与
//! work / session 几层共用同一份实现。本插件只提供落位与地址，配置在这里的作用是
//! **把闸门取值喂给共享实现**。

use serde::{Deserialize, Serialize};

/// agent 插件配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// 智能体自身的 `AGENTS.md`（两个作用域共用）的**写入**字节上限。
    #[serde(default = "default_memory_max_bytes")]
    pub memory_max_bytes: usize,

    /// 智能体自身的 `AGENTS.md`（两个作用域共用）注入系统提示词的**字节**上限。
    #[serde(default = "default_memory_inject_max_bytes")]
    pub memory_inject_max_bytes: usize,
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
            memory_max_bytes: default_memory_max_bytes(),
            memory_inject_max_bytes: default_memory_inject_max_bytes(),
        }
    }
}

impl AgentConfig {
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
