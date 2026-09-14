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
//! 工具不认识能力管理器、不认识挂载点、不走协议信封——地址翻译、按挂载名选
//! provider、workdir 透传全部在 [`ToolVdfs`] 内，与前端链路（`super::super::host`）
//! 共用同一批注册的 provider，不存在第二套实现。
//!
//! ## 地址规则（三类，实现在 `ToolVdfs`）
//!
//! 1. **本地文件地址**（无 `.vdfs` 前缀）→ `local/xxx`：裸地址即会话工作目录
//!    地址空间，挂载点名字与 local 插件注册名共用 [`PLUGIN_LOCAL`] 常量；
//! 2. **虚拟目录 `.vdfs`**：系统资源类别**挂接**在此目录之下；`vdfs_list('.vdfs')`
//!    即可枚举当前可访问的全部类别（不再提供独立的挂载点清单工具）；
//! 3. **虚拟地址**（`.vdfs/<挂载名>/…`）→ 剥掉前缀，首段 = 在 `CapabilityVisitor`
//!    里注册的挂载名，直调对应 provider。
//!
//! ## provider 从哪来
//!
//! 工具注册广播（`traverse`）的 ctx 携带 `CAPABILITY_VISITOR`；`plugin.rs` 在那次
//! 广播中构造 [`ToolVdfs::new(visitor)`] 并把工具注册进同一个 visitor。执行时
//! visitor 里已注册好全部挂载 provider（容器同时注册的组合根供前端链路使用）。

pub mod list;
pub mod tree;
pub mod stat;
pub mod read;
pub mod edit;
pub mod search;
pub mod write;
pub mod delete;
pub mod mkdir;
pub mod r#move;

use crate::symbio_core::{
    Capability, CapabilityCategory, CapabilityMeta, InvokeRequest, InvokeRequestExt,
    ToolContextRetention,
};
use serde::de::DeserializeOwned;
use std::sync::Arc;

/// 工具链路的封装 provider（地址翻译 + 按挂载名直调，见 `super::super::provider`）
pub use super::provider::ToolVdfs;

/// 读取请求载荷为具体请求类型（缺省容忍空载荷）
pub(crate) fn request_of<T: DeserializeOwned + Default>(ctx: &Arc<dyn InvokeRequest>) -> T {
    ctx.payload::<serde_json::Value>()
        .ok()
        .and_then(|v| serde_json::from_value::<T>(v).ok())
        .unwrap_or_default()
}

/// 统一的路径参数说明（所有工具的 schema 共享同一套语义，避免 LLM 误用）
///
/// 与原生本地文件工具（`read_file` 等）**同一套地址规则**：相对路径从工作目录开始。
pub const PATH_DESC: &str =
    "路径（本地工作目录）。相对路径从工作目录开始，如 'README.md'、'src/main.rs'；'/' 表示工作目录根；绝对路径直用，如 'D:/tmp/a.txt'。系统资源类别统一挂接在虚拟目录 '.vdfs' 之下：'.vdfs/<挂载名>/...' 访问对应类别（如 '.vdfs/setting/appearance'）；对 '.vdfs' 本身列目录即可枚举当前可访问的全部类别。";

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
    use crate::symbio_core::vdfs_provider::VFDS_PARAM_WORKDIR;
    use crate::symbio_core::{DefaultToolVisitor, SimpleRequest, WORKDIR};

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
                .get(VFDS_PARAM_WORKDIR)
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
        let names: Vec<String> =
            vdfs_tools(empty_provider()).iter().map(|t| t.name()).collect();
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
