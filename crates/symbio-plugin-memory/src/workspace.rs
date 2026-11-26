//! 工作区记忆的**落位与地址** —— 机制在共享实现（`memory_file`）。
//!
//! 本插件同时管两个作用域的记忆（见 [`super`] 的模块文档），本模块是**工作区**那
//! 一腿的个性；智能体自身那一腿见 [`super::memory`]。
//!
//! ## 落位：工作区根目录的 `MEMORY.md`
//!
//! ```text
//! {workdir}/MEMORY.md    工作区记忆本体（三层统一物理名，随仓库可版本化）
//! ```
//!
//! 三层记忆物理名统一 `MEMORY.md`（会话 / 工作区 / 智能体），机制一致、可预测。
//! 它与工作区的**指令文件 `AGENTS.md` 语义隔离**：后者归 `setting` 插件读取注入
//! （稳定指令），本文件归 memory（动态记忆）。两者各由不同所有者经作用域 provider
//! 读写，物理隔离。
//!
//! ## 挂载名 ≠ 物理名：`WORKSPACE.md`
//!
//! 智能体记忆与工作区记忆**同住 `<根>/memory` 一个挂载点**，而两者的物理文件同名
//! （都叫 `MEMORY.md`）——同一个列表里出现两个 `MEMORY.md`，模型和用户都分不清
//! 「这份记忆是谁的」。因此挂载里的名字用 [`MOUNT_FILE`]（`WORKSPACE.md`）：
//! 名字自带作用域。物理文件名是三层统一的 `MEMORY.md`。
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
//! ## 归属：这一层只归本插件
//!
//! 同一份 `{workdir}/MEMORY.md` 不重复注入，否则同一内容进两次上下文而模型改不动它。
//! 约定是**谁能读写它，谁负责注入它**：本插件是工作区与智能体两个作用域的唯一所有者。
//!
//! ## 本模块与共享实现的分工
//!
//! | 谁 | 负责 |
//! |---|---|
//! | `memory_file` | 读写、两道容量闸门、片段排版、VDFS 节点形状 |
//! | 本模块 | 落位、挂载名、VDFS 地址、片段标题、空内容提示 |
//! | [`super::config`] | 两道闸门各开多大 |
//!
//! 于是这里不再有第二份「超限怎么办」「截断怎么算」——那些口径全项目只有一份。

use crate::memory_file::{MemoryFile, MemoryNodeSpec, MemorySegmentSpec};

/// 工作区记忆的**物理**文件名 —— `{workdir}/MEMORY.md`（三层统一）。
///
/// 与指令文件 `AGENTS.md` 语义隔离：后者归 `setting`（稳定指令，两级覆盖）。
/// 挂载里的名字见 [`MOUNT_FILE`]（两者刻意不同）。
pub const WORK_MEMORY_FILE: &str = "MEMORY.md";

/// 工作区记忆在**本插件挂载点**里的名字。
///
/// 与智能体记忆（物理名同为 `MEMORY.md`）同住一个挂载点，挂载名必须能区分
/// 「这份记忆是谁的」——见模块文档「挂载名 ≠ 物理名」。
pub const MOUNT_FILE: &str = "WORKSPACE.md";

/// 系统提示词条目在收集器里的注册名（同名覆盖的键）。
///
/// 子树里的注册会经作用域访问器加上 `agent/<id>/` 前缀，
/// 因此与系统侧的**同名**条目不冲突（并集）。
pub const SEGMENT_NAME: &str = "workspace-memory";

/// 片段的标题（渲染为 `【工作区记忆】`）
pub const SEGMENT_TITLE: &str = "工作区记忆";

/// 记忆文件的语义描述（VDFS 节点与插件元数据共用）
pub const MEMORY_DESCRIPTION: &str =
    "本工作区的长期记忆（跨会话保留）：用户偏好、项目约定、已定结论、环境事实。";

/// 由工作区作用域 provider 构造记忆门面（`None` = 没选工作区）。
///
/// 作用域闸门从「目录字符串是否为空」改为「provider 是否存在」——
/// 两者等价，但后者不再让插件认识物理目录。两道容量闸门仍在此注入。
pub fn store(
    provider: Option<symbio_plugin_api::DynVdfsProvider>,
    write_max_bytes: usize,
    inject_max_bytes: usize,
) -> MemoryFile {
    MemoryFile::via(
        provider,
        WORK_MEMORY_FILE,
        write_max_bytes,
        inject_max_bytes,
    )
}

/// 工作区记忆在**本插件挂载点之下**的相对路径（就是挂载名 [`MOUNT_FILE`]）。
///
/// 相对地址是常态：provider 全程只跟相对地址打交道。需要协议级绝对地址的场合
/// （提示词里印给模型的可编辑地址），由调用方经
/// `symbio_platform::sdk::vdfs::absolute_addr(ctx, rel)` 用上下文的当前父地址拼出——
/// 挂载点叫什么不归本插件。
pub fn rel_path() -> &'static str {
    MOUNT_FILE
}

/// VDFS 节点规格 —— `list` 与 `stat` 共用，两条链路不会分叉。
///
/// `name` 必须覆盖成 [`MOUNT_FILE`]：节点名若取真实文件名（`MEMORY.md`），
/// 会与智能体记忆在同一个列表里撞名。
pub fn node_spec() -> MemoryNodeSpec<'static> {
    MemoryNodeSpec {
        title: SEGMENT_TITLE,
        name: Some(MOUNT_FILE),
        kind: "workspace",
        description: MEMORY_DESCRIPTION,
    }
}

/// 片段规格 —— 本层的「个性」只有三样：标题、地址、空内容时说什么。
///
/// `address` 由调用方算好传入（`address` 返回 `String`，不能借给返回值长期持有）。
/// 排版（一行头信息 + 正文 + 空 / 截断提示）由共享实现
/// `MemoryFile::segment` 统一决定。
pub fn segment_spec(address: &str) -> MemorySegmentSpec<'_> {
    MemorySegmentSpec {
        title: SEGMENT_TITLE,
        address,
        // 两个作用域同住一个挂载点，点明互属关系：写混了模型会找错地方
        note: Some("全工作区共享，与【智能体记忆】相互独立"),
        empty_hint: "暂无内容，可写入本工作区长期有效的约定（偏好、结论、环境事实）",
    }
}

#[cfg(test)]
#[path = "workspace.test.rs"]
mod tests;
