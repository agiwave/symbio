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
//! [`UnifiedFs`](super::super::fs::UnifiedFs) 一处，与前端链路（`super::super::host`）
//! 共用同一份实现，不存在第二套。
//!
//! ## 地址规则（实现在 `UnifiedFs`）
//!
//! 1. **虚拟目录 `.vdfsv2`**：系统资源类别统一挂接在此目录之下；`vdfs_list('.vdfsv2')`
//!    即可枚举当前可访问的全部类别；
//! 2. **其余地址 = 物理磁盘**：相对路径从工作目录开始（`README.md`），绝对路径
//!    直用（`D:/tmp/a.txt`）。
//!
//! ## provider 从哪来
//!
//! 工具注册广播（`traverse`）的 ctx 携带 `CAPABILITY_VISITOR`；`plugin.rs` 在那次
//! 广播中构造 [`ToolVdfs::new(visitor)`] 并把工具注册进同一个 visitor。执行时
//! visitor 里已注册好全部挂载 provider（容器同时注册的组合根供前端链路使用）。

pub mod delete;
pub mod edit;
pub mod list;
pub mod mkdir;
pub mod r#move;
pub mod read;
pub mod search;
pub mod stat;
pub mod tree;
pub mod write;

use crate::symbio_core::{Capability, CapabilityCategory, CapabilityMeta, ToolContextRetention};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;

/// 工具链路的封装 provider（地址翻译 + 按挂载名直调，见 `super::super::provider`）
pub use super::provider::ToolVdfs;

/// 把**已拆好的调用参数**解析为具体请求类型（缺省容忍空载荷）。
///
/// 参数由 `symbio_core::invoke_capability` 从信封拆出后作为入参给出，
/// 因此这里不再自己回读 `ctx.payload()`——「拆信封」只有一处。
pub(crate) fn request_of<T: DeserializeOwned + Default>(args: &Value) -> T {
    serde_json::from_value::<T>(args.clone()).unwrap_or_default()
}

/// 统一的路径参数说明（所有工具的 schema 共享同一套语义，避免 LLM 误用）
///
/// 与后端统一地址规则同一口径：`.vdfsv2` 打头 = 系统资源，其余 = 磁盘文件。
pub const PATH_DESC: &str =
    "路径（统一文件系统）。磁盘文件：相对路径从工作目录开始，如 'README.md'、'src/main.rs'；'/' 表示工作目录根；绝对路径直用，如 'D:/tmp/a.txt'。系统资源：以 '.vdfsv2' 打头，列 '.vdfsv2' 可枚举全部资源类别，'.vdfsv2/<类别>/...' 深入对应资源（如 '.vdfsv2/setting/appearance'）。";

/// 构造工具元数据骨架；子模块在自身 `meta()` 里直接调用，避免重复样板。
pub fn tool(
    name: &str,
    description: &str,
    parameters: serde_json::Value,
    examples: Vec<&str>,
    retention: Option<ToolContextRetention>,
) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: parameters,
        category: Some(CapabilityCategory::Resource),
        examples: Some(examples.into_iter().map(str::to_string).collect()),
        context_retention: retention,
        keywords: vec![
            "资源".to_string(),
            "文件".to_string(),
            "路径".to_string(),
            "vdfs".to_string(),
        ],
    }
}

/// 构造全部 VDFS 工具：每个工具持有**同一个**封装 provider（无状态，可复用）
pub fn vdfs_tools(provider: Arc<ToolVdfs>) -> Vec<Arc<dyn Capability>> {
    vec![
        Arc::new(list::ListTool::new(provider.clone())),
        Arc::new(tree::TreeTool::new(provider.clone())),
        Arc::new(stat::StatTool::new(provider.clone())),
        Arc::new(read::ReadTool::new(provider.clone())),
        Arc::new(edit::EditTool::new(provider.clone())),
        Arc::new(search::SearchTool::new(provider.clone())),
        Arc::new(write::WriteTool::new(provider.clone())),
        Arc::new(delete::DeleteTool::new(provider.clone())),
        Arc::new(mkdir::MkdirTool::new(provider.clone())),
        Arc::new(r#move::MoveTool::new(provider)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::vdfs_provider::VDFS_PARAM_WORKDIR;
    use crate::symbio_core::{
        DefaultToolVisitor, InvokeRequest, InvokeRequestExt, SimpleRequest, WORKDIR,
    };

    /// workdir 由宿主 ctx 翻译成 provider 参数；缺省时不带该键
    ///
    /// 这是前端链路与工具链路**共用**的翻译（`host::call_params`），封装 provider
    /// 依赖它把工作目录透传给 local provider——契约若被破坏，本地相对路径立即失效。
    #[test]
    fn workdir_is_translated_to_param() {
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        assert!(super::super::host::call_params(&ctx).is_empty());

        ctx.set(WORKDIR, "/tmp/ws".to_string());
        assert_eq!(
            super::super::host::call_params(&ctx)
                .get(VDFS_PARAM_WORKDIR)
                .and_then(|v| v.as_str()),
            Some("/tmp/ws")
        );
    }

    fn empty_provider() -> Arc<ToolVdfs> {
        Arc::new(ToolVdfs::new(Arc::new(DefaultToolVisitor::new())))
    }

    #[test]
    fn metas_are_llm_ready() {
        for t in vdfs_tools(empty_provider()) {
            let m = t.meta();
            assert!(!m.description.is_empty());
            assert_eq!(m.category, Some(CapabilityCategory::Resource));
            assert!(m.input_schema.get("type").is_some());
            // examples 会经 description_for_llm 追加给 LLM
            assert!(m.description_for_llm().contains("示例"));
        }
    }

    /// 每个工具名与操作一一对应（工具集完整性）
    #[test]
    fn tools_cover_all_ops() {
        let names: Vec<String> = vdfs_tools(empty_provider())
            .iter()
            .map(|t| t.name())
            .collect();
        assert_eq!(
            names,
            vec![
                "vdfs_list",
                "vdfs_tree",
                "vdfs_stat",
                "vdfs_read",
                "vdfs_edit",
                "vdfs_search",
                "vdfs_write",
                "vdfs_delete",
                "vdfs_mkdir",
                "vdfs_move",
            ]
        );
    }
}
