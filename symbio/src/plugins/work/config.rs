//! 工作区记忆插件的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! 三个字段对应本插件的三件事：**注不注**（开关）、**能写多大**（写入闸门）、
//! **每轮背多少**（注入闸门）。默认值即本插件出厂行为，字段真源就是本结构——
//! 表单定义从 [`WorkConfig::default`] 读出，不写第二份字面量（与 `local` 同一口径，
//! 避免「面板显示值与实际行为不符」的漂移）。

use serde::{Deserialize, Serialize};

/// 工作区记忆配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkConfig {
    /// 是否向系统提示词注入工作区记忆。
    ///
    /// 关掉只影响**注入**：文件与 VDFS 挂载点照旧可用（用户仍能在界面上编辑）。
    #[serde(default = "default_true")]
    pub memory_enabled: bool,

    /// 记忆文件**单次写入**的字节上限（硬限制，超出拒绝写入）。
    #[serde(default = "default_max_bytes")]
    pub memory_max_bytes: usize,

    /// 每轮请求**注入**的记忆正文字节上限（超出部分截断，靠地址读取全文）。
    ///
    /// 与上一条是两道独立的闸门：记忆文件可以比注入预算大。
    #[serde(default = "default_inject_max_bytes")]
    pub memory_inject_max_bytes: usize,
}

fn default_true() -> bool {
    true
}

/// 16 KiB：够写几十条长期事实，又不至于让一次写入把工作区文件撑爆
fn default_max_bytes() -> usize {
    16 * 1024
}

/// 4 KiB：约一千余汉字，占一次请求上下文的比重很小
fn default_inject_max_bytes() -> usize {
    4 * 1024
}

impl Default for WorkConfig {
    fn default() -> Self {
        Self {
            memory_enabled: true,
            memory_max_bytes: default_max_bytes(),
            memory_inject_max_bytes: default_inject_max_bytes(),
        }
    }
}

impl WorkConfig {
    /// 生效的写入上限（下界 1 字节，避免配置成 0 后一切写入都失败却看不出原因）
    pub fn effective_max_bytes(&self) -> usize {
        self.memory_max_bytes.max(1)
    }

    /// 生效的注入预算（下界 1 字节，理由同上）
    pub fn effective_inject_bytes(&self) -> usize {
        self.memory_inject_max_bytes.max(1)
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
