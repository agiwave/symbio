//! VDFS 工具集 —— 把 VDFS 暴露给大语言模型
//!
//! ## 每个工具 = 一个框架原生 `Capability`，持有封装的 provider
//!
//! 本目录下的每一个文件（`list.rs` / `read.rs` / …）定义一个**框架原生的
//! `Capability` 实现**——与已下线的原生 local 工具（`read_file` / `file_edit` /
//! `dir_list` / `glob_search` / `write_file` / `delete_file`）是同一种东西：
//!
//! - `meta()` 返回工具的 LLM 元数据（名称 / 描述 / 入参 schema / 示例 / 风险类别）；
//! - `execute()` 做两件事——① **对文件系统的操作改为对 `ToolVdfs` 的操作**
//!   （构造时注入），② 把域响应封装成大模型更易消费的形状（行号、ignore 过滤、
//!   人类可读 `message` …）。
//!
//! 工具不认识能力管理器、不认识目录拓扑、不走协议信封——地址规则（`.vdfsv2`
//! 前缀 = 虚拟，其余 = 磁盘）、两半分流、workdir 透传全部在 [`ToolVdfs`] 背后的
//! `UnifiedFs` 一处，与前端链路（`super::host`）
//! 共用同一份实现，不存在第二套。
//!
//! ## 地址规则（实现在 `UnifiedFs`）
//!
//! 1. **虚拟目录 `.vdfsv2`**：系统资源类别统一挂接在此目录之下；`vdfs_list('.vdfsv2')`
//!    即可枚举当前可访问的全部类别；
//! 2. **其余地址 = 物理磁盘**：相对路径从工作目录开始（`README.md`），绝对路径照
//!    操作系统解析（首部的 `/` 是绝对锚点：`/docs` 是文件系统根下的 docs，
//!    `<盘符>:/tmp/a.txt` 是那个盘上的文件）——`docs` 与 `/docs` 是两个位置。
//!
//! ## provider 从哪来
//!
//! 工具注册广播（`traverse`）的 ctx 携带 `CAPABILITY_VISITOR`；`plugin.rs` 在那次
//! 广播中构造 [`ToolVdfs::new`] 并把工具注册进同一个 visitor。执行时
//! visitor 里已注册好全部挂载 provider（容器同时注册的组合根供前端链路使用）。
//!
//! ## 同形工具走表（[`spec`]）
//!
//! 九个工具里，**七个**是「转发到 provider 的某个方法 + 包一层回执」的同形体，
//! 差异只有「名字 / 描述 / schema / 示例 / 保留策略 / 调哪个方法」六样。
//! 它们由 [`spec::SPECS`] 一张表驱动，`Capability` 实现**只有一份**
//! （[`spec::VdfsTool`]）——不再一个工具一个文件。
//!
//! `read`（行号 + 分页）与 `list`（ignore glob 过滤 + 目录优先排序）带
//! **表装不下的呈现加工**，保留为独立文件（判据是「有没有真逻辑」，不是行数）。

pub mod list;
pub mod read;
pub mod spec;

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;
use symbio_plugin_api::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, PluginError,
};

/// 工具链路的封装 provider（地址翻译 + 按挂载名直调，见 `super::super::provider`）
pub use super::provider::ToolVdfs;

/// 把**已拆好的调用参数**解析为具体请求类型（缺省容忍空载荷）。
///
/// 参数由 `symbio_plugin_api::capability_invoke` 从信封拆出后作为入参给出，
/// 因此这里不再自己回读 `ctx.payload()`——「拆信封」只有一处。
pub(crate) fn request_of<T: DeserializeOwned + Default>(args: &Value) -> T {
    serde_json::from_value::<T>(args.clone()).unwrap_or_default()
}

/// 校验 schema 声明为 `required` 的**字符串**参数确实存在且非空。
///
/// `request_of` 的 `unwrap_or_default` 会把缺失字段吞成空串/空结构——空 path
/// 落到物理层恰好解析成工作目录根本身，产生「'…' 是目录」这类与真实
/// 原因无关的误导性错误（实测会话 `09d74431`：LLM 在超限上下文下吐出空参数，
/// `vdfs_read` 收到 `{}` 后一路静默走到物理层才炸）。schema 说了 required，
/// 执行侧就要兑现它。
pub(crate) fn ensure_required(args: &Value, field: &str) -> Result<(), PluginError> {
    let present = args
        .get(field)
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    if present {
        Ok(())
    } else {
        Err(PluginError::ValidationError(format!(
            "缺少必填参数: {field}"
        )))
    }
}

/// 统一的路径参数说明（所有工具的 schema 共享同一套语义，避免 LLM 误用）
///
/// 与后端统一地址规则同一口径（实现见 `super::super::fs` 的 `normalize_addr` /
/// `super::super::physical` 的 `join_target`）。这份文案要说清的是**两件事各自的写法**：
/// 相对段从工作目录起算（`.` 是工作目录根）、首部的 `/` 是操作系统的绝对锚点
/// （`/docs` ≠ `docs`）。裁决与后果见
/// [D-172](../../../../docs/003-design/009-VDFS规范.md#d-172-首段--是绝对锚点不是可折叠的分隔符docs-相对工作目录docs-是文件系统里的-docs "DE-009-VDFS规范")。
pub const PATH_DESC: &str =
    "路径（统一文件系统）。磁盘文件有两种写法，**不是同一个位置**：① 相对工作目录——'README.md'、'src/main.rs'、'tmp/note.md' 都落在工作目录之下，工作目录根本身写成 '.'；② 绝对路径——首部的 '/' 是操作系统的绝对锚点，'/docs' 指文件系统根下的 docs（POSIX 的 /docs，Windows 落在工作目录所在盘的根目录下），带盘符的 'D:/tmp/a.txt' 同理。因此 '/docs' 与 'docs' 是两个位置：前者在文件系统根，后者在工作目录里；想把工作目录下的 docs 当成绝对路径交给工具，请写 '.' 再加名字，不要照抄一条外部机器的绝对路径。系统资源：以 '.vdfsv2' 打头，列 '.vdfsv2' 可枚举全部资源类别，'.vdfsv2/<类别>/...' 深入对应资源（如 '.vdfsv2/plugin_manager/appearance'）。";

/// 构造工具元数据骨架；子模块在自身 `meta()` 里直接调用，避免重复样板。
///
/// `risk` 由调用方**显式**给出而非走默认档：这几个工具同属一个插件、schema 形状
/// 几乎一样，但风险并不相同（`vdfs_read` 只读、`vdfs_write` 改状态）。让默认值
/// 隐式决定，等于把「哪些工具能改状态」这件事藏进了一个看不见的兜底里。
pub fn tool(
    name: &str,
    description: &str,
    parameters: serde_json::Value,
    examples: Vec<&str>,
    risk: CapabilityRiskLevel,
    behaviors: &'static [&'static str],
) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: parameters,
        category: Some(CapabilityCategory::Resource),
        examples: Some(examples.into_iter().map(str::to_string).collect()),
        risk: Some(risk),
        // 行为名**显式**给（空集也要写）：`vdfs_write` 一族声明 `file_change`，
        // 其余工具一个都不声明。省略字段就退化成「默认无行为」，而默认值正是
        // 「忘了声明」与「确定不要」不可分的那种形状（DE-025 D-210）。
        behaviors,
    }
}

/// 构造全部 VDFS 工具：每个工具持有**同一个**封装 provider（无状态，可复用）。
///
/// 顺序固定（`tools_cover_all_ops` 测试锁死）：表装不下的两个（`list` / `read`）
/// 手动插入到它们在原清单里的位置，其余七条由 [`spec::SPECS`] 按表序展开。
pub fn vdfs_tools(provider: Arc<ToolVdfs>) -> Vec<Arc<dyn Capability>> {
    let by_name = |name: &str| -> Arc<dyn Capability> {
        let s = spec::SPECS
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("规格表缺少工具 {name}"));
        Arc::new(spec::VdfsTool::new(provider.clone(), s))
    };

    vec![
        Arc::new(list::ListTool::new(provider.clone())), // 1. vdfs_list
        by_name("vdfs_tree"),                            // 2. vdfs_tree
        by_name("vdfs_stat"),                            // 3. vdfs_stat
        Arc::new(read::ReadTool::new(provider.clone())), // 4. vdfs_read
        by_name("vdfs_edit"),                            // 5. vdfs_edit
        by_name("vdfs_search"),                          // 6. vdfs_search
        by_name("vdfs_write"),                           // 7. vdfs_write
        by_name("vdfs_delete"),                          // 8. vdfs_delete
        by_name("vdfs_mkdir"),                           // 9. vdfs_mkdir
    ]
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio-plugin-vdfs/src/tools/mod.rs::vdfs_tools: 穷尽性不变量：上游已穷举 / 已校验，走到 else 说明本文件的判据漏了一个分支，属代码缺陷。
