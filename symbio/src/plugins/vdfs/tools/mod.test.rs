//! `symbio/src/plugins/vdfs/tools/mod.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::VDFS_PARAM_WORKDIR;
use crate::symbio_core::{
    DefaultToolVisitor, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, WORKDIR,
};

/// workdir 由宿主 ctx 翻译成 provider 参数；缺省时不带该键
///
/// 这是前端链路与工具链路**共用**的翻译（`host::call_params`），封装 provider
/// 依赖它把工作目录透传给 local provider——契约若被破坏，本地相对路径立即失效。
#[test]
fn workdir_is_translated_to_param() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
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
///
/// **没有 `vdfs_move`**：移动整条下线了（见 `VdfsRequest` 的「没有 `Move`」一节）
/// ——核心层不收「两个地址」的操作，要用移动由外层组合，当前外层没提供。
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
        ]
    );
}
