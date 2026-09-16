//! 智能体记忆的**落位与地址** —— 机制在内核（`symbio_core::memory`）。
//!
//! ## 落位：bundle 自己的目录
//!
//! ```text
//! <本插件目录>/<bundle_id>/AGENTS.md           全局级 bundle 的记忆
//! {workdir}/<本插件目录>/<bundle_id>/AGENTS.md    工作区级 bundle 的记忆
//! ```
//!
//! 记忆**跟着智能体走**，不跟着工作区走：同一个智能体在 A 项目与 B 项目里记得同一批事。
//! 工作区根的 `{workdir}/AGENTS.md` 是另一件事（由 work 插件管）——两者**同名不同作用域**，
//! 所以片段里必须点明「该智能体私有，与【工作区记忆】相互独立」，否则模型会把两件事写混。
//!
//! ## 作用域闸门：**选中了智能体才有记忆**
//!
//! `ctx[AGENT_ID]` 缺失 / 为空 = 本次会话没选智能体 → 人格与记忆都不注册（分支在
//! [`super::plugin::AgentPlugin::attach_bundle`]）。bundle 不存在（目录被删 / 被改名）
//! 同样构造出「无作用域」的 [`MemoryFile`]：读 → 明确报错、注入 → `None`、写 → 明确报错，
//! 调用点不需要各自判断。
//!
//! ## 归属：这一层**只归本插件**
//!
//! 三层记忆（工作区 / 会话 / 智能体）各自只有一个所有者。判据是
//! **谁能读写它，谁负责注入它** —— 它同时消灭「重叠」（同一份文件被两个插件各注入一次，
//! 同一内容进两次上下文）与「模糊」（同一文件一处叫「指令」只读、一处叫「记忆」可写，
//! 用户看不出该往哪写）。
//!
//! ## 本模块与内核的分工
//!
//! | 谁 | 负责 |
//! |---|---|---|
//! | `symbio_core::memory` | 读写、两道容量闸门、片段排版、VDFS 节点形状 |
//! | 本模块 | 落位、VDFS 地址、片段标题、空内容提示 |
//! | [`super::config`] | 两道闸门各开多大 |
//!
//! 于是本插件不再自己写「超限怎么办」「截断怎么算」——那些口径全项目只有一份。
//! ⚠️ **人格不走内核**：`prompts/` + `skills/` 装配出的是**多文件形态**，不是单文件记忆，
//! 由 [`super::prompt`] 自己排版（但形态与内核对齐：一行头信息 + 正文 + 空 / 截断提示）。

use super::store::BundleStore;
use crate::symbio_core::{MemoryFile, NodeSpec, SegmentSpec, AGENTS_FILE, PLUGIN_AGENT};

/// 片段的标题（渲染为 `【智能体记忆】`）
pub const SEGMENT_TITLE: &str = "智能体记忆";

/// 系统提示词条目在收集器里的注册名（同名覆盖的键）
pub const SEGMENT_NAME: &str = "agent-memory";

/// 记忆文件的语义描述（VDFS 节点与插件元数据共用）
pub const MEMORY_DESCRIPTION: &str =
    "该智能体自己的长期记忆（跨会话保留）：只在选中本智能体时注入系统提示词。";

/// 智能体记忆文件的 **VDFS 展示地址**（下发给模型的可编辑地址）。
///
/// 与其它资源同一口径：展示地址以 `.vdfs` 开头，线路协议仍是 `/…`；
/// 翻译只发生在 `tauri/src/services/vdfs.ts`。
pub fn memory_address(bundle_id: &str) -> String {
    format!(".vdfs/{PLUGIN_AGENT}/{bundle_id}/{AGENTS_FILE}")
}

/// 由 bundle 记录构造记忆门面（bundle 不存在 → **无作用域**）。
///
/// 两道闸门在此注入：内核只认「上限是多少」，不关心它从哪个配置来。
/// 作用域判断也在此一次收口，之后读 / 注入 / 写三条路各自降级。
pub fn store(
    bundles: &BundleStore,
    bundle_id: &str,
    write_max_bytes: usize,
    inject_max_bytes: usize,
) -> MemoryFile {
    match bundles.memory_path(bundle_id) {
        Ok(path) => MemoryFile::new(Some(path), write_max_bytes, inject_max_bytes),
        Err(_) => MemoryFile::absent(write_max_bytes, inject_max_bytes),
    }
}

/// 片段规格 —— 本层的「个性」只有三样：标题、地址、空内容时说什么。
///
/// 排版（一行头信息 + 正文 + 空 / 截断提示）由内核
/// [`render_segment`](crate::symbio_core::render_segment) 统一决定。
pub fn segment_spec(address: &str) -> SegmentSpec<'_> {
    SegmentSpec {
        title: SEGMENT_TITLE,
        address,
        // 与 work 的工作区记忆同名不同域：不点明的话模型会把两件事写混
        note: Some("该智能体私有，与【工作区记忆】相互独立"),
        empty_hint: "暂无记忆，可写入该智能体跨会话应记住的偏好、结论、教训",
    }
}

/// VDFS 节点规格 —— `list` 与 `stat` 共用，两条链路不会分叉。
pub fn node_spec() -> NodeSpec<'static> {
    NodeSpec {
        title: SEGMENT_TITLE,
        // 场景标签与另外两层同一口径：用**所属插件**的场景名，而不是另造一个 `memory`
        // （`memory` 在前端未登记任何图标 / 渲染器，等于一个没有消费者的死标签）
        kind: PLUGIN_AGENT,
        description: MEMORY_DESCRIPTION,
    }
}

#[cfg(test)]
#[path = "memory.test.rs"]
mod tests;
