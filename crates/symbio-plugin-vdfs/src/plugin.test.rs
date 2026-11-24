//! `crates/symbio-plugin-vdfs/src/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use symbio_plugin_api::PluginSimpleRequest;

fn build_plugin() -> Arc<dyn Plugin> {
    VdfsPlugin::build(Arc::new(PluginSimpleRequest::new(None, None)))
}

fn ctx_with_path(path: &str) -> Arc<dyn PluginInvokeRequest> {
    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, path.to_string());
    ctx
}

#[test]
fn tool_path_facts_resolve_physical_addresses_but_preserve_virtual_root() {
    assert_eq!(
        path_intent_fact(r"src\main.rs", r"C:\work\repo"),
        symbio_plugin_api::SandboxFact::new("vdfs_path", "c:/work/repo/src/main.rs")
    );
    assert_eq!(
        path_intent_fact(".vdfsv2/sandbox/strict", "/workspace"),
        symbio_plugin_api::SandboxFact::new("vdfs_path", ".vdfsv2/sandbox/strict")
    );
    assert_eq!(
        path_intent_fact("/etc/passwd", "/workspace"),
        symbio_plugin_api::SandboxFact::new("vdfs_path", "/etc/passwd")
    );
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
    let visitor: Arc<dyn symbio_plugin_api::Registry> =
        Arc::new(symbio_providers::DefaultToolVisitor::new());
    let names: Vec<String> = tools::vdfs_tools(Arc::new(ToolVdfs::new(visitor)))
        .iter()
        .map(|(m, _)| m.name.clone())
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

/// 静态声明可达（本 crate 编译进了 vdfs 插件，它提交了根名）。
/// 具体的名字归本插件——这里只钉「声明机制在工作」，并把归一化结果钉在常量上。
#[test]
fn declaration_is_reachable_and_normalized() {
    let root = symbio_plugin_api::descend_addr("", "");
    assert_eq!(root, VDFS_ADDR_ROOT);
    assert!(
        !root.ends_with('/') && !root.starts_with('/'),
        "应已归一化: {root}"
    );
}

/// 转发跳的改写规则：顶层落到声明的根；嵌套时在当前父地址上续接
#[test]
fn descend_starts_at_root_then_layers() {
    // 顶层（转发方自己没有父地址）→ 根 + 本级名
    assert_eq!(
        symbio_plugin_api::descend_addr("", "session"),
        format!("{VDFS_ADDR_ROOT}/session")
    );
    assert_eq!(
        symbio_plugin_api::descend_addr("  ", "session"),
        format!("{VDFS_ADDR_ROOT}/session")
    );
    // 嵌套（转发方已有父地址）→ 原地续接，不回落到根
    assert_eq!(
        symbio_plugin_api::descend_addr("@elsewhere/agent/b1", "sub"),
        "@elsewhere/agent/b1/sub"
    );
}
