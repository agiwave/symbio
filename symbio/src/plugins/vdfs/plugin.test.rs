//! `symbio/src/plugins/vdfs/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::PluginSimpleRequest;

fn build_plugin() -> Arc<dyn Plugin> {
    VdfsPlugin::build(Arc::new(PluginSimpleRequest::new(None, None)))
}

fn ctx_with_path(path: &str) -> Arc<dyn PluginInvokeRequest> {
    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, path.to_string());
    ctx
}

#[tokio::test]
async fn route_unknown_path_is_not_found() {
    let err = build_plugin()
        .route(ctx_with_path("bogus"))
        .await
        .unwrap_err();
    assert!(matches!(err, PluginError::NotFound(_)));
}

/// 无父插件（未挂载）时虚拟根照样可列出：资源集合由容器注册决定，
/// 宿主自身不持有任何资源——「没有容器」与「资源为空」表现一致。
#[tokio::test]
async fn list_vdfs_root_without_parent_is_empty() {
    let ctx = ctx_with_path("list");
    ctx.set_payload(serde_json::json!({ "path": ".vdfsv2" }))
        .unwrap();
    let resp = build_plugin().route(ctx).await.unwrap();
    let data = resp.get::<p::VdfsListResponse>().unwrap();
    assert_eq!(data.path, ".vdfsv2");
    assert!(data.items.is_empty());
}

/// 未知子目录 → NotFound（提示现有子目录）
#[tokio::test]
async fn list_unknown_dir_errors() {
    let ctx = ctx_with_path("list");
    ctx.set_payload(serde_json::json!({ "path": ".vdfsv2/nope" }))
        .unwrap();
    let err = build_plugin().route(ctx).await.unwrap_err();
    assert!(matches!(err, PluginError::NotFound(_)));
}

/// 工具集：每个 VDFS 操作恰好一个工具（无 `vdfs_move`——移动已整条下线）
#[test]
fn tools_cover_all_ops() {
    let visitor: Arc<dyn crate::symbio_core::CapabilityVisitor> =
        Arc::new(crate::providers::collectors::DefaultToolVisitor::new());
    let names: Vec<String> = tools::vdfs_tools(Arc::new(ToolVdfs::new(visitor)))
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
