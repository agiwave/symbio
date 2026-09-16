//! `work/vdfs.rs` 的单元测试 —— 挂载点的寻址、闸门与不可删除。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use crate::symbio_core::vdfs::vdfs_context;
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, PluginDir, SimpleRequest, WORKDIR};
use std::sync::Arc;
use tempfile::TempDir;

/// 无工作区上下文的记忆插件（配置落在临时目录，不碰真实 homedir）
fn plugin_without_workspace(tmp: &TempDir) -> WorkPlugin {
    WorkPlugin::new(
        PluginDir::at(tmp.path(), "work"),
        super::super::config::WorkConfig::default(),
    )
}

/// 构造 VDFS 调用上下文；`workdir` 为 `None` 即「没有工作区」
fn vctx(workdir: Option<&str>) -> VdfsContext {
    let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    if let Some(w) = workdir {
        ctx.set(WORKDIR, w.to_string());
    }
    vdfs_context(&ctx)
}

// ==================== 寻址 ====================

#[tokio::test]
async fn root_node_leaves_its_name_to_the_caller() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    let root = p.stat(&vctx(None), "").await.unwrap();
    assert_eq!(root.name, "", "provider 不知道自己被挂在哪里");
    assert!(root.is_dir());
    assert_eq!(root.access, VdfsAccess::LIST_TRAVERSE);
    assert!(p.root_new_types().is_empty(), "根下不可新建");
}

#[tokio::test]
async fn root_lists_the_memory_file_and_nothing_else() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    let ws = TempDir::new().unwrap();
    let nodes = p
        .list(&vctx(Some(ws.path().to_string_lossy().as_ref())), "")
        .await
        .unwrap();

    assert_eq!(nodes.len(), 1, "根下只列记忆文件（配置文档不并列）");
    let n = &nodes[0];
    assert_eq!(n.name, AGENTS_FILE);
    assert_eq!(n.access, VdfsAccess::READ_WRITE);
    assert_eq!(
        n.effective_ext().as_deref(),
        Some("md"),
        "文件名即呈现扩展名"
    );
    assert_eq!(n.kind, PLUGIN_WORK);
}

#[tokio::test]
async fn no_workspace_lists_nothing() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    assert!(p.list(&vctx(None), "").await.unwrap().is_empty());
    // 没有工作区 = 记忆这个资源不存在，而不是「空文件」
    assert!(p.stat(&vctx(None), AGENTS_FILE).await.is_err());
}

#[tokio::test]
async fn config_document_is_reachable_by_its_real_file_name() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    let node = p.stat(&vctx(None), PLUGIN_FILE).await.unwrap();
    assert_eq!(node.name, PLUGIN_FILE);
    assert!(node.access.read && node.access.write);
    // 但不在根列表里（列表只列业务内容）
    let ws = TempDir::new().unwrap();
    let listed = p
        .list(&vctx(Some(ws.path().to_string_lossy().as_ref())), "")
        .await
        .unwrap();
    assert!(listed.iter().all(|n| n.name != PLUGIN_FILE));
}

#[tokio::test]
async fn unknown_paths_are_not_found() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    let ws = TempDir::new().unwrap();
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));
    assert!(p.list(&c, "sub").await.is_err());
    assert!(p.stat(&c, "nope.md").await.is_err());
    assert!(p.read(&c, "nope.md").await.is_err());
}

// ==================== 读写 ====================

#[tokio::test]
async fn missing_memory_reads_as_empty() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    let ws = TempDir::new().unwrap();
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));

    let content = p.read(&c, AGENTS_FILE).await.unwrap();
    assert_eq!(content.text.as_deref(), Some(""), "还没有记忆 ≠ 读失败");
}

#[tokio::test]
async fn write_then_read_roundtrips_and_reports_creation() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    let ws = TempDir::new().unwrap();
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));

    let first = p
        .write(&c, AGENTS_FILE, &VdfsContent::text(AGENTS_FILE, "偏好中文"))
        .await
        .unwrap();
    assert!(first.created, "首次写入即创建");

    let again = p
        .write(
            &c,
            AGENTS_FILE,
            &VdfsContent::text(AGENTS_FILE, "偏好中文 + 简洁"),
        )
        .await
        .unwrap();
    assert!(!again.created, "覆盖写入不是创建");

    let content = p.read(&c, AGENTS_FILE).await.unwrap();
    assert_eq!(content.text.as_deref(), Some("偏好中文 + 简洁"));
    assert_eq!(content.size, "偏好中文 + 简洁".len() as u64);
}

/// 写入闸门：超限**拒绝**，且不留下半截内容
#[tokio::test]
async fn oversized_write_is_rejected_by_the_capacity_gate() {
    let tmp = TempDir::new().unwrap();
    let cfg = super::super::config::WorkConfig {
        memory_max_bytes: 8,
        ..super::super::config::WorkConfig::default()
    };
    let p = WorkPlugin::new(PluginDir::at(tmp.path(), "work"), cfg);
    let ws = TempDir::new().unwrap();
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));

    p.write(&c, AGENTS_FILE, &VdfsContent::text(AGENTS_FILE, "12345678"))
        .await
        .unwrap();
    let err = p
        .write(
            &c,
            AGENTS_FILE,
            &VdfsContent::text(AGENTS_FILE, "123456789"),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, VdfsError::Invalid(_)),
        "应为校验失败：{err:?}"
    );
    assert!(err.to_string().contains("超出容量上限"), "{err}");
    assert_eq!(
        p.read(&c, AGENTS_FILE).await.unwrap().text.as_deref(),
        Some("12345678"),
        "被拒绝的写入不得改动文件"
    );
}

#[tokio::test]
async fn binary_write_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    let ws = TempDir::new().unwrap();
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));
    let err = p
        .write(
            &c,
            AGENTS_FILE,
            &VdfsContent::binary(AGENTS_FILE, "AA==", 1),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::Invalid(_)));
}

// ==================== 不可删除 ====================

#[tokio::test]
async fn delete_is_forbidden_but_clearing_by_write_is_allowed() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_without_workspace(&tmp);
    let ws = TempDir::new().unwrap();
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));
    p.write(&c, AGENTS_FILE, &VdfsContent::text(AGENTS_FILE, "内容"))
        .await
        .unwrap();

    let err = p.delete(&c, AGENTS_FILE, false).await.unwrap_err();
    assert!(matches!(err, VdfsError::Forbidden(_)), "{err:?}");
    assert!(p.delete(&c, "", false).await.is_err(), "挂载点不可删");

    // 清空的正确姿势：写入空内容
    p.write(&c, AGENTS_FILE, &VdfsContent::text(AGENTS_FILE, ""))
        .await
        .unwrap();
    assert_eq!(
        p.read(&c, AGENTS_FILE).await.unwrap().text.as_deref(),
        Some("")
    );
}
