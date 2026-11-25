//! memory 插件的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! 本插件同时管**三个作用域**的记忆，配置也按作用域分组：
//!
//! | 作用域 | 字段 | 闸门 | 超限行为 |
//! |---|---|---|---|
//! | 智能体自身（分形：随实例走） | `agent_enabled` / `memory_max_bytes` / `memory_inject_max_bytes` | `MemoryFile::write` / `MemoryFile::inject` | **拒绝** / **截断** + 告知地址 |
//! | 工作区（会话作用域 `ctx[WORKDIR]`） | `workspace_enabled` / `workspace_max_bytes` / `workspace_inject_max_bytes` | 同上（同一份共享实现） | 同上 |
//! | 会话（`ctx[SESSION_DIR]`） | `session_enabled` / `session_max_bytes` / `session_inject_max_bytes` | 同上（同一份共享实现） | 同上 |
//!
//! 注入为什么比写入小得多：注入的正文**每一轮**都在烧上下文（4 KiB），
//! 写入是一次性的（16 KiB，超出部分模型按地址 `vdfs_read` 取全文）。
//!
//! 为什么写侧是拒绝：记忆是**跨会话生效**的东西，写入被截断意味着模型以为改好了、
//! 实际少了一块——这种失败没有任何报错，只能靠「拒绝」把它变成一次显式的、可重试的失败。
//!
//! 三层记忆各有独立的**注入开关**（`agent_enabled` / `workspace_enabled` /
//! `session_enabled`）：用户可按作用域决定是否把该层记忆注入提示词。关掉只影响
//! **注入**，文件与挂载条目照旧可用（用户仍能在界面上读写）。三层机制一致，开关
//! 语义也一致。
//!
//! ## 配置跟着**实例**走，不跟着插件走
//!
//! 系统树实例读 `<homedir>/memory/PLUGIN.yml`，子智能体实例读
//! `<agentdir>/memory/PLUGIN.yml`——分形的必然结果：两个作用域各有各的落位，
//! 也就各有各的配置。给两个作用域硬凑「共用一对闸门」需要一份跨实例的全局配置，
//! 那正是分形要避免的东西。
//!
//! 读写与闸门本身住在同一 crate 的 `memory_file` 私有模块，三层共用；本插件只提供
//! 落位与地址，配置在这里的作用是**把闸门取值喂给该模块**。

use serde::{Deserialize, Serialize};

/// memory 插件配置（三个作用域各一组闸门与开关）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    // —— 智能体自身记忆（分形：随实例落在各自宿主目录） ——
    /// 是否向系统提示词注入智能体记忆。
    ///
    /// 关掉只影响**注入**：文件与挂载条目照旧可用。
    #[serde(default = "default_true")]
    pub agent_enabled: bool,

    /// 智能体自身的 `MEMORY.md` 的**写入**字节上限（硬限制，超出拒绝写入）。
    #[serde(default = "default_max_bytes")]
    pub memory_max_bytes: usize,

    /// 智能体自身的 `MEMORY.md` 注入系统提示词的**字节**上限（超出部分截断并指路）。
    #[serde(default = "default_inject_max_bytes")]
    pub memory_inject_max_bytes: usize,

    // —— 工作区记忆（会话作用域：ctx[WORKDIR]，物理文件 `{workdir}/MEMORY.md`） ——
    /// 是否向系统提示词注入工作区记忆。
    ///
    /// 关掉只影响**注入**：文件与挂载条目照旧可用（用户仍能在界面上编辑）。
    #[serde(default = "default_true")]
    pub workspace_enabled: bool,

    /// 工作区记忆文件**单次写入**的字节上限（硬限制，超出拒绝写入）。
    #[serde(default = "default_max_bytes")]
    pub workspace_max_bytes: usize,

    /// 每轮请求**注入**的工作区记忆正文字节上限（超出部分截断，靠地址读取全文）。
    ///
    /// 与上一条是两道独立的闸门：记忆文件可以比注入预算大。
    #[serde(default = "default_inject_max_bytes")]
    pub workspace_inject_max_bytes: usize,

    // —— 会话记忆（会话作用域：ctx[SESSION_DIR]，物理文件 `{session_dir}/MEMORY.md`） ——
    /// 是否向系统提示词注入会话记忆。
    ///
    /// 关掉只影响**注入**：文件与挂载条目照旧可用。
    #[serde(default = "default_true")]
    pub session_enabled: bool,

    /// 会话记忆文件**单次写入**的字节上限（硬限制，超出拒绝写入）。
    #[serde(default = "default_max_bytes")]
    pub session_max_bytes: usize,

    /// 每轮请求**注入**的会话记忆正文字节上限（超出部分截断，靠地址读取全文）。
    #[serde(default = "default_inject_max_bytes")]
    pub session_inject_max_bytes: usize,
}

fn default_true() -> bool {
    true
}

/// 16 KiB：两道写入闸门同一口径（两者是同一类东西，只是作用域不同）——
/// 够写几十条长期事实，又不至于让一次写入把文件撑爆
fn default_max_bytes() -> usize {
    16 * 1024
}

/// 4 KiB：约一千余汉字，占一次请求上下文的比重很小
fn default_inject_max_bytes() -> usize {
    4 * 1024
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            agent_enabled: true,
            memory_max_bytes: default_max_bytes(),
            memory_inject_max_bytes: default_inject_max_bytes(),
            workspace_enabled: true,
            workspace_max_bytes: default_max_bytes(),
            workspace_inject_max_bytes: default_inject_max_bytes(),
            session_enabled: true,
            session_max_bytes: default_max_bytes(),
            session_inject_max_bytes: default_inject_max_bytes(),
        }
    }
}

impl MemoryConfig {
    /// 生效的智能体记忆写入上限（下界 1 字节，避免配成 0 后一切写入都失败却看不出原因）
    pub fn effective_memory_max_bytes(&self) -> usize {
        self.memory_max_bytes.max(1)
    }

    /// 生效的智能体记忆注入上限（下界 1 字节，理由同上）
    pub fn effective_memory_inject_bytes(&self) -> usize {
        self.memory_inject_max_bytes.max(1)
    }

    /// 生效的工作区记忆写入上限（下界 1 字节，理由同上）
    pub fn effective_workspace_max_bytes(&self) -> usize {
        self.workspace_max_bytes.max(1)
    }

    /// 生效的工作区记忆注入上限（下界 1 字节，理由同上）
    pub fn effective_workspace_inject_bytes(&self) -> usize {
        self.workspace_inject_max_bytes.max(1)
    }

    /// 生效的会话记忆写入上限（下界 1 字节，理由同上）
    pub fn effective_session_max_bytes(&self) -> usize {
        self.session_max_bytes.max(1)
    }

    /// 生效的会话记忆注入上限（下界 1 字节，理由同上）
    pub fn effective_session_inject_bytes(&self) -> usize {
        self.session_inject_max_bytes.max(1)
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
