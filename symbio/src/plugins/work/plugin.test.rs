//! `work/plugin.rs` 的单元测试 —— 收集期交出的两样东西与**作用域闸门**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 重点钉住：**选了工作区才有记忆**（没选工作区时什么都不注入），以及开关关掉
//! 只影响注入、不影响 VDFS 挂载点。

use super::*;
use crate::providers::DefaultToolVisitor;
use crate::symbio_core::{CapabilityVisitor, PluginInvokeRequestExt, VDFS_PARENT_ADDR};
use tempfile::TempDir;

/// 构造带能力收集器的上下文；`workdir` 为 `None` 即「没选工作区」
fn ctx(workdir: Option<&str>) -> (Arc<dyn PluginInvokeRequest>, Arc<DefaultToolVisitor>) {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    if let Some(w) = workdir {
        ctx.set(WORKDIR, w.to_string());
    }
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    // 合成父地址：模拟容器转发时写入的当前父地址（不依赖真实挂载名）
    ctx.set(VDFS_PARENT_ADDR, "@vfs/work".to_string());
    let visitor = Arc::new(DefaultToolVisitor::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&visitor) as Arc<dyn CapabilityVisitor>,
    );
    (ctx, visitor)
}

fn plugin(tmp: &TempDir, config: WorkConfig) -> Arc<WorkPlugin> {
    Arc::new(WorkPlugin::new(
        crate::symbio_core::PluginDir::at(tmp.path(), PLUGIN_ID_WORK),
        config,
    ))
}

/// 工作区 + 一份 AGENTS.md
fn workspace_with_memory(text: &str) -> TempDir {
    let ws = TempDir::new().unwrap();
    std::fs::write(ws.path().join(WORK_MEMORY_FILE), text).unwrap();
    ws
}

// ==================== 作用域闸门 ====================

#[tokio::test]
async fn no_workspace_injects_nothing() {
    let tmp = TempDir::new().unwrap();
    let p = plugin(&tmp, WorkConfig::default());
    let (ctx, visitor) = ctx(None);

    p.traverse(String::new(), ctx).await.unwrap();

    assert!(
        visitor.list_system_prompts().await.is_empty(),
        "没选工作区时不得注入任何记忆"
    );
    // 但挂载点照旧交出：挂载点的存在不依赖某次请求的作用域
    assert!(visitor.get_vdfs_provider(PLUGIN_ID_WORK).await.is_some());
}

#[tokio::test]
async fn empty_workspace_still_injects_the_empty_hint() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    let p = plugin(&tmp, WorkConfig::default());
    let (ctx, visitor) = ctx(Some(ws.path().to_string_lossy().as_ref()));

    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].0, SEGMENT_NAME);
    assert!(
        segs[0].1.contains("暂无内容"),
        "空记忆也要教会模型怎么建立记忆：{}",
        segs[0].1
    );
    assert!(segs[0].1.contains("@vfs/work/AGENTS.md"));
}

#[tokio::test]
async fn workspace_memory_is_injected_verbatim() {
    let tmp = TempDir::new().unwrap();
    let ws = workspace_with_memory("本仓库用中文提交信息。");
    let p = plugin(&tmp, WorkConfig::default());
    let (ctx, visitor) = ctx(Some(ws.path().to_string_lossy().as_ref()));

    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(segs.len(), 1);
    assert!(segs[0].1.contains("本仓库用中文提交信息。"));
    assert!(segs[0].1.contains("@vfs/work/AGENTS.md"));
    assert!(segs[0].1.contains("【工作区记忆】"));
}

#[tokio::test]
async fn disabled_memory_skips_injection_but_keeps_the_mount() {
    let tmp = TempDir::new().unwrap();
    let ws = workspace_with_memory("会被忽略的内容");
    let cfg = WorkConfig {
        memory_enabled: false,
        ..WorkConfig::default()
    };
    let p = plugin(&tmp, cfg);
    let (ctx, visitor) = ctx(Some(ws.path().to_string_lossy().as_ref()));

    p.traverse(String::new(), ctx).await.unwrap();

    assert!(visitor.list_system_prompts().await.is_empty());
    assert!(
        visitor.get_vdfs_provider(PLUGIN_ID_WORK).await.is_some(),
        "关掉注入不等于关掉编辑面"
    );
}

// ==================== 无自有路由 ====================

#[tokio::test]
async fn plugin_has_no_own_routes() {
    let tmp = TempDir::new().unwrap();
    let p = plugin(&tmp, WorkConfig::default());
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    ctx.set(PATH, "work/whatever".to_string());
    let err = p.route(ctx).await.unwrap_err();
    assert!(
        format!("{err:?}").contains("VDFS"),
        "错误信息要指引到 VDFS：{err:?}"
    );
}
