//! 会话记忆 —— **本会话自己的** `AGENTS.md`（机制在内核 `symbio_core::memory`）。
//!
//! ## 落位与地址
//!
//! ```text
//! <本插件目录>/<会话 id>/AGENTS.md     记忆本体
//! <父地址>/<会话 id>/AGENTS.md      可编辑地址（父地址 = 上下文里的当前父地址）
//! ```
//!
//! 与 `session.json` / `messages.json` 同一目录——它就是这个会话的一部分，
//! 随会话一起删除。
//!
//! ## 它跟另外两层记忆是不是重复了？
//!
//! 值得说清楚，因为「会话记忆」这个名字容易被当成多余的一层：
//!
//! | 层 | 记什么 | 活多久 |
//! |---|---|---|
//! | 工作区 | 这个项目的约定、环境事实 | 跟着工作区 |
//! | 智能体 | 这个智能体跨会话的偏好与教训 | 跟着智能体 |
//! | **会话** | **本会话自己钉住的约定与结论** | **跟着会话** |
//!
//! 转写（`messages.json`）已经记了「说过什么」，但它是**流水**：轮次一多，早期的
//! 结论会被压缩、淡化、乃至淘汰出上下文。会话记忆是**从这个会话里提炼出来的、
//! 不许被压缩掉的那几条**——它的价值恰恰在于「不随上下文水位消失」。
//!
//! 因此它的定位是**本会话的置顶约定**，不是「对话摘要」（那是压缩快照的活），
//! 也不是「跨会话的经验」（那该写进工作区或智能体记忆）。
//!
//! ## 作用域闸门：**有会话 id 才有记忆**
//!
//! `ctx[SESSION_ID]` 缺失 / 为空 → 什么都不注入。收集期拿不到会话 id 的广播
//!（例如设置页的选项收集）不该凭空造一份记忆出来。

use crate::symbio_core::{MemoryFile, MemorySegmentSpec, MEMORY_AGENTS_FILE};
use std::path::PathBuf;

/// 系统提示词条目在收集器里的注册名（同名覆盖的键）
pub const SEGMENT_NAME: &str = "session-memory";

/// 条目的标题（渲染为 `【会话记忆】`）
pub const SEGMENT_TITLE: &str = "会话记忆";

/// 记忆文件的语义描述（VDFS 节点与插件元数据共用）
pub const MEMORY_DESCRIPTION: &str =
    "本会话自己的长期约定（跨轮次保留，不被上下文压缩淘汰）：钉住的结论、约束、待办。";

/// 会话记忆文件：`<会话目录>/AGENTS.md`
pub fn memory_path(root: &std::path::Path, session_id: &str) -> PathBuf {
    super::paths::session_dir(root, session_id).join(MEMORY_AGENTS_FILE)
}

/// 记忆在 **provider 子树内**的相对路径（`<id>/AGENTS.md`）。
///
/// 相对地址是常态：provider 全程只跟相对地址打交道（变更发射用它构造
/// `VdfsChange::path`，与 `list` 返回的节点地址严格一致）。需要协议级绝对地址
/// 的场合（提示词里印给模型的可编辑地址），由调用方经
/// `symbio_core::vdfs::absolute_addr(ctx, rel)` 用上下文的当前父地址拼出——
/// 挂载点叫什么不归本插件。
pub fn memory_rel_path(session_id: &str) -> String {
    format!("{session_id}/{MEMORY_AGENTS_FILE}")
}

/// 由会话 id 构造记忆门面（`None` / 空串 / 纯空白 = 无会话作用域）。
///
/// 两道闸门在此注入：内核只认「上限是多少」，不关心它从哪个配置来。
pub fn store(
    root: &std::path::Path,
    session_id: Option<&str>,
    write_max_bytes: usize,
    inject_max_bytes: usize,
) -> MemoryFile {
    let path = session_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|id| memory_path(root, id));
    MemoryFile::new(path, write_max_bytes, inject_max_bytes)
}

/// 条目规格 —— 本层的「个性」只有三样：标题、地址、空内容时说什么。
///
/// `address` 是**绝对地址**（调用方经 `absolute_addr` 从上下文父地址拼出，
/// 返回 `String`，不能借给返回值长期持有）。排版由内核
/// [`memory_render_segment`](crate::symbio_core::memory_render_segment) 统一决定。
pub fn segment_spec(address: &str) -> MemorySegmentSpec<'_> {
    MemorySegmentSpec {
        title: SEGMENT_TITLE,
        address,
        // 三层记忆同名不同域，不点明的话模型会把三件事写混
        note: Some("本会话私有，与【工作区记忆】【智能体记忆】相互独立"),
        empty_hint: "暂无内容，可写入本会话不许被压缩掉的约定与结论",
    }
}

#[cfg(test)]
#[path = "memory.test.rs"]
mod tests;
