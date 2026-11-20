//! 会话记忆的**落位与地址** —— 机制在共享实现（`providers/memory`）。
//!
//! 本插件的三个记忆作用域同构：`agent_dir` / `work_dir` / `session_dir`。本模块是
//! `session_dir` 那一腿的个性；另外两腿见 [`super::memory`] / [`super::workspace`]。
//!
//! ## 落位：会话数据目录的 `MEMORY.md`
//!
//! ```text
//! {session_dir}/MEMORY.md    记忆本体（物理文件）
//! <根>/memory/SESSION.md     可编辑地址（挂载名）
//! ```
//!
//! `session_dir` 由会话插件按自己的唯一存储布局解析，经上下文 `SESSION_DIR` 递来
//! （D-513）。本模块只把现成目录与文件名相接：**不**认识 session id、**不**复现
//! 会话目录派生规则、**不**拿这个目录拼事实源路径（D-084 / FB-002）。
//!
//! ## 为什么挂载名不是 `MEMORY.md`
//!
//! 三层记忆同住 `<根>/memory`，地址本身要能说出作用域：智能体那份叫 `AGENTS.md`、
//! 工作区那份叫 `WORKSPACE.md`，会话这份于是叫 `SESSION.md`。物理文件名保留
//! `MEMORY.md`——它是会话目录里的长期约定，不是 agent 目录里的指令文件。
//!
//! ## 作用域闸门：**收得到会话目录才有记忆**
//!
//! `ctx[SESSION_DIR]` 缺失 / 为空 = 当前上下文没有会话作用域 → 什么都不注入、列表
//! 里也不出现。这与工作区腿同一纪律；收集期读不到就没凭空造一份出来。

use symbio_providers::{MemoryFile, MemoryNodeSpec, MemorySegmentSpec};

/// 会话记忆的**物理**文件名 —— `{session_dir}/MEMORY.md`。
pub const SESSION_MEMORY_FILE: &str = "MEMORY.md";

/// 会话记忆在**本插件挂载点**里的名字。
pub const MOUNT_FILE: &str = "SESSION.md";

/// 系统提示词条目在收集器里的注册名。
pub const SEGMENT_NAME: &str = "session-memory";

/// 片段标题（渲染为 `【会话记忆】`）。
pub const SEGMENT_TITLE: &str = "会话记忆";

/// 记忆文件的语义描述（VDFS 节点与插件元数据共用）。
pub const MEMORY_DESCRIPTION: &str =
    "本会话自己的长期约定（跨轮次保留，不被上下文压缩淘汰）：钉住的结论、约束、待办。";

/// 由会话作用域 provider 构造记忆门面（`None` = 无会话作用域）。
///
/// D-515：作用域闸门从「目录字符串是否为空」改为「provider 是否存在」。
/// 两道容量闸门在此注入；本插件不认识 `SESSION_ID`，也不从它拼目录。
pub fn store(
    provider: Option<symbio_plugin_api::DynVdfsProvider>,
    write_max_bytes: usize,
    inject_max_bytes: usize,
) -> MemoryFile {
    MemoryFile::via(
        provider,
        SESSION_MEMORY_FILE,
        write_max_bytes,
        inject_max_bytes,
    )
}

/// 会话记忆在**本插件挂载点之下**的相对路径（就是挂载名 [`MOUNT_FILE`]）。
pub fn rel_path() -> &'static str {
    MOUNT_FILE
}

/// VDFS 节点规格 —— `list` 与 `stat` 共用，两条链路不会分叉。
pub fn node_spec() -> MemoryNodeSpec<'static> {
    MemoryNodeSpec {
        title: SEGMENT_TITLE,
        name: Some(MOUNT_FILE),
        kind: "session",
        description: MEMORY_DESCRIPTION,
    }
}

/// 片段规格 —— 本层的「个性」只有标题、地址、区分说明与空内容提示。
pub fn segment_spec(address: &str) -> MemorySegmentSpec<'_> {
    MemorySegmentSpec {
        title: SEGMENT_TITLE,
        address,
        note: Some("本会话私有，与【工作区记忆】【智能体记忆】相互独立"),
        empty_hint: "暂无内容，可写入本会话不许被压缩掉的约定与结论",
    }
}

#[cfg(test)]
#[path = "session.test.rs"]
mod tests;
