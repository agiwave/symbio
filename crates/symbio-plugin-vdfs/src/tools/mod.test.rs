//! `crates/symbio-plugin-vdfs/src/tools/mod.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use symbio_plugin_api::collectors::DefaultToolVisitor;
use symbio_plugin_api::VDFS_PARAM_WORKDIR;
use symbio_plugin_api::{
    PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, WORKDIR,
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
    for (m, _, _) in vdfs_tools(empty_provider()) {
        assert!(!m.description.is_empty());
        assert_eq!(m.category, Some(CapabilityCategory::Resource));
        assert!(m.input_schema.get("type").is_some());
        // examples 由协议适配层（model）追加给 LLM —— 此处只校验「声明了 examples」，
        // 拼接格式归 model 的 `description_for_llm` 自己测（core 不再承载该格式）。
        assert!(m.examples.as_ref().is_some_and(|e| !e.is_empty()));
    }
}

#[test]
fn mutating_tools_declare_approval_targets_and_effects() {
    let expected = [
        ("vdfs_edit", "file_modify"),
        ("vdfs_write", "file_modify"),
        ("vdfs_delete", "file_delete"),
        ("vdfs_mkdir", "file_modify"),
    ];
    for (meta, _, approval) in vdfs_tools(empty_provider()) {
        if let Some((_, effect)) = expected.iter().find(|(name, _)| *name == meta.name) {
            let summary = approval
                .unwrap_or_else(|| panic!("{} 缺少审批摘要", meta.name))
                .summary(&serde_json::json!({"path": "tmp/x"}));
            assert_eq!(summary["effect"], *effect);
            assert_eq!(summary["target"], "tmp/x");
        } else {
            assert!(approval.is_none(), "{} 无需修改状态", meta.name);
        }
    }
}

/// LLM 工具面固定名单：`vdfs_list` 保留逐层列举，递归树只存在于宿主协议。
///
/// **没有 `vdfs_move`**：移动整条下线了（见 `VdfsRequest` 的「没有 `Move`」一节）
/// ——核心层不收「两个地址」的操作，要用移动由外层组合，当前外层没提供。
#[test]
fn tools_cover_all_ops() {
    let names: Vec<String> = vdfs_tools(empty_provider())
        .iter()
        .map(|(m, _, _)| m.name.clone())
        .collect();
    assert_eq!(
        names,
        vec![
            "vdfs_list",
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
