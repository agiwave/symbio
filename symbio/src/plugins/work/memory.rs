//! 工作区记忆的**落位与地址** —— 机制在内核（`symbio_core::memory`）。
//!
//! ## 落位：工作区根目录的 `AGENTS.md`
//!
//! ```text
//! {workdir}/AGENTS.md                        工作区记忆（唯一文件，行业通行约定）
//! <本插件目录>/PLUGIN.yml          本插件配置（两道闸门开多大）
//! ```
//!
//! 文件名与位置**刻意对齐行业惯例**（`AGENTS.md`：给编码智能体的工作区级指令文件，
//! 与 `CLAUDE.md` / `.cursorrules` 同一族）。收益是记忆**不属于 symbio**：
//!
//! - 换任何支持该约定的工具，这份记忆照样生效——它不是某个宿主的私有数据；
//! - 它就躺在工作区里，能被 `git` 版本化、能被 review、能被其它协作者读到；
//! - 用户不需要知道 symbio 的数据目录在哪就能编辑它。
//!
//! ## 作用域闸门：**选了工作区才有记忆**
//!
//! `ctx[WORKDIR]` 缺失 / 为空 = 用户没有在工作区上下文里提问 → **什么都不注入**。
//! 这不是优化，是语义：「工作区记忆」四个字里，作用域是它的一部分；把 A 项目的
//! 约定带到 B 项目、或者带到一个根本没选工作区的闲聊会话里，都是错的。
//!
//! 闸门落在 [`store`]：没有工作目录就构造出一个「无作用域」的 [`MemoryFile`]，
//! 之后读 / 注入 / 片段三条路各自降级，调用点不需要重复判断。
//!
//! ## 归属：这一层**只归本插件**
//!
//! 同一个 `{workdir}/AGENTS.md` 曾经被 session 插件也读一遍（当「工作区指令」注入
//! 系统提示词基座），结果是同一份内容进两次上下文、而模型改不动它。现在按
//! **谁能读写它，谁负责注入它**收口——session 不再碰这个文件。
//!
//! ## 本模块与内核的分工
//!
//! | 谁 | 负责 |
//! |---|---|---|
//! | `symbio_core::memory` | 读写、两道容量闸门、片段排版、VDFS 节点 |
//! | 本模块 | 落位、VDFS 地址、片段标题、空内容提示 |
//! | [`super::config`] | 两道闸门各开多大 |
//!
//! 于是本插件不再自己写「超限怎么办」「截断怎么算」——那些口径全项目只有一份。

use crate::symbio_core::{MemoryFile, SegmentSpec, AGENTS_FILE};
use std::path::{Path, PathBuf};

/// 记忆文件的 **VDFS 展示地址**（下发给模型的可编辑地址）。
///
/// 与其它资源同一口径：展示地址以 `.vdfs` 开头，线路协议仍是 `/…`；
/// 翻译只发生在 `tauri/src/services/vdfs.ts`。
pub const MEMORY_VDFS_ADDRESS: &str = ".vdfs/work/AGENTS.md";

/// 片段的标题（渲染为 `【工作区记忆】`）
pub const SEGMENT_TITLE: &str = "工作区记忆";

/// 系统提示词条目在收集器里的注册名（同名覆盖的键）
pub const SEGMENT_NAME: &str = "work-memory";

/// 记忆文件的语义描述（VDFS 节点与插件元数据共用）
pub const MEMORY_DESCRIPTION: &str =
    "本工作区的长期记忆（跨会话保留）：用户偏好、项目约定、已定结论、环境事实。";

/// 工作区记忆文件：`<workdir>/AGENTS.md`
pub fn memory_path(workdir: &str) -> PathBuf {
    Path::new(workdir).join(AGENTS_FILE)
}

/// 由工作目录构造记忆门面（`None` / 空串 / 纯空白 = 无工作区）。
///
/// 两道闸门在此注入：内核只认「上限是多少」，不关心它从哪个配置来。
pub fn store(workdir: Option<&str>, write_max_bytes: usize, inject_max_bytes: usize) -> MemoryFile {
    let path = workdir
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(memory_path);
    MemoryFile::new(path, write_max_bytes, inject_max_bytes)
}

/// 片段规格 —— 本层的「个性」只有三样：标题、地址、空内容时说什么。
///
/// 排版（一行头信息 + 正文 + 空 / 截断提示）由内核
/// [`render_segment`](crate::symbio_core::render_segment) 统一决定。
pub fn segment_spec() -> SegmentSpec<'static> {
    SegmentSpec {
        title: SEGMENT_TITLE,
        address: MEMORY_VDFS_ADDRESS,
        // 工作区记忆没有需要区分的邻居（智能体记忆在它自己的地址下），故不加附加说明
        note: None,
        empty_hint: "暂无内容，可写入本工作区长期有效的约定（偏好、结论、环境事实）",
    }
}

#[cfg(test)]
#[path = "memory.test.rs"]
mod tests;
