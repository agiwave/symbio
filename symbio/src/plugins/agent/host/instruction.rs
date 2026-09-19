//! 智能体自身的 `AGENTS.md` —— **agent 插件管的第二个作用域**。
//!
//! ## 为什么是 agent 插件
//!
//! `AGENTS.md` 是「**某个智能体自身的指令**」。项目里有两个这样的作用域：
//!
//! | 作用域 | 目录 | 文件 | 生效范围 |
//! |---|---|---|---|
//! | 系统智能体 | `<homedir>` | `{homedir}/AGENTS.md` | 所有会话 |
//! | 子智能体 | `<agentdir>` | `<agentdir>/AGENTS.md` | 选中该智能体时 |
//!
//! 两者都归**智能体域**，而这个域整体归 `agent` 插件：它已经在管智能体目录
//! （扫描、装配子树、整包浏览 `agent/<id>/…`）。指令文件就在那些目录里，
//! 由它一起管，读写面与注入面因此落在**同一个插件**上——
//! 「谁能读写它，谁负责注入它」这条原则重新完整成立。
//!
//! > 这件事一度被拆成两半：`{homedir}/AGENTS.md` 由 `session` 临时读一遍注入
//! > （叫「全局指令」，只读、无地址、无容量），`<agentdir>/AGENTS.md` 则由子树里
//! > 那个被宿主改指了作用域的 `work` 实例混在「工作区记忆」名下注入（还带着
//! > 工作区的地址）。同一件事两个所有者、两种口径。
//! >
//! > 后来它也一度被收进 `setting`（理由是「智能体自身的设置」），但那是错的：
//! > `setting` 的分工是**设置页的入口**（自有分区 + 各插件的配置清单），
//! > 它不该同时成为某一份内容文件的所有者。指令归 `agent`，入口继续归 `setting`。
//!
//! ## 系统智能体目录 = 本插件目录的**父目录**
//!
//! 容器的装配规则是「一层目录 = 一个插件，插件并列在智能体目录下」，因此
//! **本插件目录的父目录就是它所在的那个智能体自身目录**——不需要任何新的上下文键，
//! 也不需要宿主额外告知：本插件在 `<homedir>/agent` → 系统智能体目录 = `<homedir>`。
//!
//! ## 与 [`super::memory`] 的分工
//!
//! | 模块 | 管哪个作用域 | 地址 |
//! |---|---|---|
//! | [`super::memory`] | 子智能体：`<agentdir>/AGENTS.md`（随 bundle 分发） | `<根>/agent/<id>/AGENTS.md` |
//! | 本模块 | 系统智能体：`{homedir}/AGENTS.md`（宿主应用级设置） | `<根>/agent/AGENTS.md` |
//!
//! 两者共用内核（`symbio_core::memory`）的读写、两道闸门、片段排版与节点形状——
//! 一份实现，两处调用。
//!
//! 空文件 / 文件不存在 → 该段整体省略（不产出空标题）：用户没写过指令时，
//! 不该让每个会话都背一段没有内容的头信息。

use crate::symbio_core::{MemoryFile, NodeSpec, SegmentSpec, AGENTS_FILE, PLUGIN_AGENT};
use std::path::{Path, PathBuf};

/// 系统提示词条目在收集器里的注册名（同名覆盖的键）
pub const SEGMENT_NAME: &str = "agent-instructions";

/// 片段的标题（渲染为 `【全局指令】`）
pub const SEGMENT_TITLE: &str = "全局指令";

/// 指令文件挂在**本插件挂载点自身**（相对地址 = 文件名 [`AGENTS_FILE`]，
/// 与智能体列表并列的一个文件）。
///
/// 需要协议级绝对地址的场合（提示词里印给模型的可编辑地址），由调用方经
/// `symbio_core::vdfs::absolute_addr(ctx, rel)` 用上下文的当前父地址拼出——
/// 挂载点叫什么不归本模块。
/// 指令文件的语义描述（VDFS 节点与插件元数据共用）
pub const DESCRIPTION: &str =
    "本应用（系统智能体）自身的指令：每轮注入系统提示词，对所有会话生效。";

/// 系统智能体自身目录 = **本插件目录的父目录**（容器的既有装配规则）
///
/// 取不到父目录（理论上不该发生）时退回本插件目录：那样作用域收窄成 bundle 目录，
/// 也不会写到别的地方去。
pub fn host_dir(plugin_dir: &Path) -> PathBuf {
    plugin_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| plugin_dir.to_path_buf())
}

/// 指令文件的落位：`{系统智能体目录}/AGENTS.md`
pub fn file_path(host_dir: &Path) -> PathBuf {
    host_dir.join(AGENTS_FILE)
}

/// 指令文件的记忆门面（内核）：两道闸门由 [`super::config::AgentConfig`] 给出
pub fn store(host_dir: &Path, write_max_bytes: usize, inject_max_bytes: usize) -> MemoryFile {
    MemoryFile::new(Some(file_path(host_dir)), write_max_bytes, inject_max_bytes)
}

/// VDFS 节点规格 —— `list` 与 `stat` 共用，两条链路不会分叉
pub fn node_spec() -> NodeSpec<'static> {
    NodeSpec {
        title: SEGMENT_TITLE,
        // 场景标签用**所属插件**的场景名（与各层同一口径），不另造一个没有消费者的标签
        kind: PLUGIN_AGENT,
        description: DESCRIPTION,
    }
}

/// 系统提示词片段：**走内核排版**（地址 + 上限 + 当前 + 正文 + 截断提示）
///
/// 本插件既暴露该地址（挂载点下的 [`AGENTS_FILE`]，绝对地址由调用方拼好传入），
/// 也执行那道写入闸门——印出来的数字是真的。
pub fn segment(store: &MemoryFile, address: &str) -> Result<Option<String>, String> {
    // 空文件 / 不存在 → 整段省略（内核的 `empty_hint` 是「空也要说一句」的用法，
    // 这里不需要：没写过指令时不该每轮都背一段头信息）
    if store.read()?.trim().is_empty() {
        return Ok(None);
    }
    store.segment(&SegmentSpec {
        title: SEGMENT_TITLE,
        address,
        note: Some("对所有会话生效"),
        empty_hint: "",
    })
}

#[cfg(test)]
#[path = "instruction.test.rs"]
mod tests;
