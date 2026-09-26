//! 智能体记忆的**落位、地址与注入** —— 机制在内核（`symbio_core::memory`）。
//!
//! ## 落位：agent 目录自己的目录
//!
//! ```text
//! <本插件目录>/<agent_id>/AGENTS.md           全局级 agent 目录的记忆
//! {workdir}/<本插件目录>/<agent_id>/AGENTS.md    工作区级 agent 目录的记忆
//! ```
//!
//! 记忆**跟着智能体走**，不跟着工作区走：同一个智能体在 A 项目与 B 项目里记得同一批事。
//! 工作区根的 `{workdir}/AGENTS.md` 是另一件事（由 work 插件管）——两者**同名不同作用域**，
//! 所以片段里必须点明「该智能体私有，与【工作区记忆】相互独立」，否则模型会把两件事写混。
//!
//! ## 作用域闸门：**选中了智能体才有记忆**
//!
//! `ctx[AGENT_ID]` 缺失 / 为空 = 本次会话没选智能体 → 不装配子树，因此也不注册任何
//! 智能体片段（分支在 [`super::plugin::AgentPlugin::traverse`]）。agent 目录不存在
//! （目录被删 / 被改名）同样构造出「无作用域」的 [`MemoryFile`]：读 → 明确报错、
//! 注入 → `None`、写 → 明确报错，调用点不需要各自判断。
//!
//! ## 归属：这一层**只归本插件**
//!
//! 各层记忆（工作区 / 会话 / 智能体自身）各自只有一个所有者。判据是
//! **谁能读写它，谁负责注入它** —— 它同时消灭「重叠」（同一份文件被两个插件各注入一次，
//! 同一内容进两次上下文）与「模糊」（同一文件一处叫「指令」只读、一处叫「记忆」可写，
//! 用户看不出该往哪写）。
//!
//! 本模块管的是**子智能体**那一份（`<agentdir>/AGENTS.md`，随 agent 目录分发）；
//! **系统智能体**那一份（`{homedir}/AGENTS.md`）归 [`super::instruction`]。
//! 两个作用域同属智能体域，因此都在本插件里——读写面与注入面于是落在同一个所有者上。
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

use super::store::AgentDirStore;
use crate::symbio_core::{
    MemoryFile, MemoryNodeSpec, MemorySegmentSpec, MEMORY_AGENTS_FILE, PLUGIN_ID_AGENT,
};

/// 系统提示词条目在收集器里的注册名（同名覆盖的键）。
///
/// 子树里的注册会经 [`super::scope::SubAgentVisitor`] 加上 `agent/<id>/` 前缀，
/// 因此与系统侧的同名条目**不冲突**（并集，§8.2）。
pub const SEGMENT_NAME: &str = "agent-memory";

/// 片段的标题（渲染为 `【智能体记忆】`）
pub const SEGMENT_TITLE: &str = "智能体记忆";

/// 记忆文件的语义描述（VDFS 节点与插件元数据共用）
pub const MEMORY_DESCRIPTION: &str =
    "该智能体自己的长期记忆（跨会话保留）：只在选中本智能体时注入系统提示词。";

/// 由 agent 目录记录构造记忆门面（agent 目录不存在 → **无作用域**）。
///
/// 两道闸门在此注入：内核只认「上限是多少」，不关心它从哪个配置来。
/// 作用域判断也在此一次收口，之后读 / 注入 / 写三条路各自降级。
pub fn store(
    agent_dirs: &AgentDirStore,
    agent_id: &str,
    write_max_bytes: usize,
    inject_max_bytes: usize,
) -> MemoryFile {
    match agent_dirs.memory_path(agent_id) {
        Ok(path) => MemoryFile::new(Some(path), write_max_bytes, inject_max_bytes),
        Err(_) => MemoryFile::absent(write_max_bytes, inject_max_bytes),
    }
}

/// VDFS 节点规格 —— `list` 与 `stat` 共用，两条链路不会分叉。
pub fn node_spec() -> MemoryNodeSpec<'static> {
    MemoryNodeSpec {
        title: SEGMENT_TITLE,
        // 场景标签与其它几层同一口径：用**所属插件**的场景名，而不是另造一个 `memory`
        // （`memory` 在前端未登记任何图标 / 渲染器，等于一个没有消费者的死标签）
        kind: PLUGIN_ID_AGENT,
        description: MEMORY_DESCRIPTION,
    }
}

/// 记忆文件在**本插件挂载点之下**的相对路径（`<id>/AGENTS.md`）。
///
/// 相对地址是常态：provider 全程只跟相对地址打交道。需要协议级绝对地址的场合
/// （提示词里印给模型的可编辑地址），由调用方经
/// `symbio_core::vdfs::absolute_addr(ctx, rel)` 用上下文的当前父地址拼出——
/// 挂载点叫什么不归本插件。文件名取 [`MEMORY_AGENTS_FILE`]：不写第二份地址。
pub fn rel_path(agent_id: &str) -> String {
    format!("{agent_id}/{MEMORY_AGENTS_FILE}")
}

/// 系统提示词片段：**走内核排版**（地址 + 上限 + 当前 + 正文 + 截断提示）。
///
/// 地址与闸门都由本插件给出、也由本插件执行（整包浏览面负责 agent 目录里所有文件的
/// 写入），所以这里是**唯一**需要印写入闸门的地方——印出来的数字是真的。
///
/// 无作用域（agent 目录不存在）→ `None`：静默跳过，不往收集期错误桶里塞东西。
pub fn segment(store: &MemoryFile, address: &str) -> Result<Option<String>, String> {
    // 空文件 / 不存在 → 整段省略（内核的 `empty_hint` 是「空也要说一句」的用法，
    // 这里不需要：没写过记忆时不该每轮都背一段头信息）
    if !store.has_scope() || store.read()?.trim().is_empty() {
        return Ok(None);
    }
    store.segment(&MemorySegmentSpec {
        title: SEGMENT_TITLE,
        address,
        // 点明与工作区那一份的关系：同名不同作用域，写混了模型会找错地方
        note: Some("本智能体私有，与【工作区记忆】相互独立"),
        empty_hint: "",
    })
}

#[cfg(test)]
#[path = "memory.test.rs"]
mod tests;
