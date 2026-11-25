//! 智能体自身记忆的**落位与地址** —— 机制在共享实现（`memory_file`）。
//!
//! ## 落位：宿主目录的 `MEMORY.md`
//!
//! ```text
//! {宿主目录}/MEMORY.md        记忆本体（宿主目录 = 本插件目录的父目录）
//! <本插件目录>/PLUGIN.yml     本插件配置（两道闸门开多大）
//! ```
//!
//! **宿主目录 = 本插件目录的父目录**，这是本插件唯一的落位规则。容器的装配规则是
//! 「一层目录 = 一个插件，插件并列在智能体目录下」，于是父目录就是**本插件所属的那个
//! 智能体自己**的目录——不需要任何新的上下文键，也不需要宿主额外告知。
//!
//! 三层记忆物理名**统一 `MEMORY.md`**（会话 / 工作区 / 智能体），机制一致、可预测。
//! 它与**指令文件 `AGENTS.md` 语义隔离**：AGENTS.md 是稳定指令 / 人格，归 `setting`
//! 插件读取并注入（工作区级 / 智能体级两级覆盖）；MEMORY.md 是动态记忆，归本插件
//! 读写可叠加。两者物理隔离，不共用文件名，也不共用所有者（D-527）。
//!
//! ## 作用域闸门：**宿主目录不存在就是没有这一层**
//!
//! 装配态下父目录一定存在（容器刚从它扫出本插件），因此本层**没有**「无作用域」的
//! 形态——这与 `work`（没选工作区就没有记忆）不同：`work` 的作用域来自会话级选择，
//! 而本插件的作用域来自**装配**，插件被构造出来这件事本身就意味着作用域存在。
//!
//! ## 归属：这一层只归本插件
//!
//! 从前这两份文件由 owner 插件用两个模块各管一个，落位规则相同却写成两份。现在它们是**同一个插件的两个
//! 实例**：读写面（`<根>/memory/…`）与注入面落在同一个所有者上，
//! 「谁能读写它，谁负责注入它」这条原则重新完整成立。
//!
//! ## 本模块与共享实现的分工
//!
//! | 谁 | 负责 |
//! |---|---|---|
//! | `memory_file` | 读写、两道容量闸门、片段排版、VDFS 节点形状 |
//! | 本模块 | 落位、VDFS 地址、片段标题、空内容提示 |
//! | [`super::config`] | 两道闸门各开多大 |
//!
//! 于是本插件不再自己写「超限怎么办」「截断怎么算」——那些口径全项目只有一份。

use crate::memory_file::{MemoryFile, MemoryNodeSpec, MemorySegmentSpec};
use symbio_plugin_api::PLUGIN_ID_MEMORY;

/// 记忆文件名 —— 三层统一 `MEMORY.md`（智能体 / 工作区 / 会话）。
///
/// 三层物理名一致，机制一致、可预测；每层各有自己的作用域 provider，因此同名
/// 不会互相干扰（各读写各目录下的 `MEMORY.md`）。
///
/// 它与指令文件 `AGENTS.md` **语义隔离**：AGENTS.md 归 `setting`（稳定指令 / 人格，
/// 两级覆盖），MEMORY.md 归本插件（动态记忆，三层叠加）。
pub const MEMORY_FILE: &str = "MEMORY.md";

/// 系统提示词条目在收集器里的注册名（同名覆盖的键）。
///
/// **不得与挂载名（插件名）同键**：4d 之后提示词行与挂载行在同一张表的同一个名字
/// 空间里（跨族同名 = 后写者胜，落盘表现是挂载点被顶掉），本插件的挂载名是 `memory`，
/// 段名于是叫 `agent-memory`——与另外两腿（`workspace-memory` / `session-memory`）同构。
///
/// 子树里的注册会经作用域访问器加上 `agent/<id>/` 前缀，
/// 因此与系统侧的**同名**条目不冲突（并集）。
pub const SEGMENT_NAME: &str = "agent-memory";

/// 插件的展示标题（元数据 / 配置表单 / 挂载根共用）。
///
/// 本插件同时管**两个作用域**的记忆（智能体自身 + 工作区），标题只叫「记忆」；
/// 各作用域自己的标题见 [`SEGMENT_TITLE`] 与 [`super::workspace::SEGMENT_TITLE`]。
pub const PLUGIN_TITLE: &str = "记忆";

/// 片段的标题（渲染为 `【智能体记忆】`）
pub const SEGMENT_TITLE: &str = "智能体记忆";

/// 记忆文件的语义描述（VDFS 节点与插件元数据共用）
pub const MEMORY_DESCRIPTION: &str =
    "本智能体自身的长期记忆（跨会话保留）：只在选中本智能体时注入系统提示词。";

/// 记忆门面（共享实现）：两道闸门由 [`super::config::MemoryConfig`] 给出。
///
/// D-515：生产形态经 `AGENT_VDFS` provider 读写，相对地址就是文件名；
/// 不再拼物理路径。「落位」用例的 `host_dir` / `file_path` helper 只存在于测试文件。
pub fn store(
    provider: Option<symbio_plugin_api::DynVdfsProvider>,
    write_max_bytes: usize,
    inject_max_bytes: usize,
) -> MemoryFile {
    MemoryFile::via(provider, MEMORY_FILE, write_max_bytes, inject_max_bytes)
}

/// VDFS 节点规格 —— `list` 与 `stat` 共用，两条链路不会分叉
pub fn node_spec() -> MemoryNodeSpec<'static> {
    MemoryNodeSpec {
        title: SEGMENT_TITLE,
        // 挂载里没有同名邻居（工作区那份叫 `WORKSPACE.md`），真实文件名即节点名
        name: None,
        kind: PLUGIN_ID_MEMORY,
        description: MEMORY_DESCRIPTION,
    }
}

/// 记忆文件在**本插件挂载点之下**的相对路径（就是文件名 [`MEMORY_FILE`]）。
///
/// 相对地址是常态：provider 全程只跟相对地址打交道。需要协议级绝对地址的场合
/// （提示词里印给模型的可编辑地址），由调用方经
/// `symbio_platform::sdk::vdfs::absolute_addr(ctx, rel)` 用上下文的当前父地址拼出——
/// 挂载点叫什么不归本插件。
pub fn rel_path() -> &'static str {
    MEMORY_FILE
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
        // 点明与工作区那一份的关系：同名不同作用域，写混了模型会找错地方
        note: Some("本智能体私有，与【工作区记忆】相互独立"),
        empty_hint: "",
    }
}

/// 系统提示词片段：**走共享实现排版**（地址 + 上限 + 当前 + 正文 + 截断提示）。
///
/// 空文件 / 文件不存在 → 整段省略：用户没写过记忆时不该每轮都背一段空头信息
/// （`empty_hint` 因此留空——它服务于「空也要说一句」的用法，本层不需要）。
///
/// 地址与闸门都印在片段里，而它们**都是本插件自己会执行的**（挂载点下的读写与
/// 注入闸门都在这里），所以印出来的数字是真的。
pub async fn segment(store: &MemoryFile, address: &str) -> Result<Option<String>, String> {
    if store.read().await?.trim().is_empty() {
        return Ok(None);
    }
    store.segment(&segment_spec(address)).await
}

#[cfg(test)]
#[path = "memory.test.rs"]
mod tests;
