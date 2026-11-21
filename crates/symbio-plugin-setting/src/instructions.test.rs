//! `instructions.rs` 的单元测试 —— 覆盖语义（工作区优先）、只读、文件名配置。

use super::*;
use std::path::Path;
use std::sync::Arc;
use symbio_plugin_api::PluginInvokeRequest;
use tempfile::TempDir;

/// 构造带两级作用域 provider 的 ctx，并按需落文件。
fn ctx_with(
    agent_dir: Option<&Path>,
    work_dir: Option<&Path>,
    file_name: &str,
    agent_body: Option<&str>,
    work_body: Option<&str>,
) -> Arc<dyn PluginInvokeRequest> {
    if let (Some(d), Some(b)) = (agent_dir, agent_body) {
        std::fs::write(d.join(file_name), b).unwrap();
    }
    if let (Some(d), Some(b)) = (work_dir, work_body) {
        std::fs::write(d.join(file_name), b).unwrap();
    }
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(symbio_plugin_api::PluginSimpleRequest::new(None, None));
    symbio_providers::attach_scope_providers(&*ctx, agent_dir, work_dir, None);
    ctx
}

#[tokio::test]
async fn workspace_instruction_overrides_agent_instruction() {
    let agent = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let ctx = ctx_with(
        Some(agent.path()),
        Some(work.path()),
        "AGENTS.md",
        Some("智能体级人格"),
        Some("工作区级人格"),
    );

    let seg = resolve(&*ctx, "AGENTS.md").await.unwrap();
    assert_eq!(seg.name, WORKSPACE_SEGMENT_NAME);
    assert_eq!(seg.title, WORKSPACE_SEGMENT_TITLE);
    assert!(seg.body.contains("工作区级人格"), "{}", seg.body);
    assert!(!seg.body.contains("智能体级人格"), "覆盖后不该再读智能体级");
    assert!(render(&seg).contains("【工作区指令】"));
}

#[tokio::test]
async fn falls_back_to_agent_instruction_when_workspace_absent() {
    let agent = TempDir::new().unwrap();
    let ctx = ctx_with(
        Some(agent.path()),
        None,
        "AGENTS.md",
        Some("智能体级人格"),
        None,
    );

    let seg = resolve(&*ctx, "AGENTS.md").await.unwrap();
    assert_eq!(seg.name, AGENT_SEGMENT_NAME);
    assert_eq!(seg.title, AGENT_SEGMENT_TITLE);
    assert!(seg.body.contains("智能体级人格"));
    assert!(render(&seg).contains("【本智能体指令】"));
}

#[tokio::test]
async fn empty_workspace_falls_back_to_agent() {
    let agent = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    // 工作区文件存在但为空 → 视作没有 → 回退智能体级
    let ctx = ctx_with(
        Some(agent.path()),
        Some(work.path()),
        "AGENTS.md",
        Some("智能体级人格"),
        Some("   \n"),
    );

    let seg = resolve(&*ctx, "AGENTS.md").await.unwrap();
    assert_eq!(seg.name, AGENT_SEGMENT_NAME);
    assert!(seg.body.contains("智能体级人格"));
}

#[tokio::test]
async fn no_instruction_anywhere_injects_nothing() {
    let agent = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let ctx = ctx_with(
        Some(agent.path()),
        Some(work.path()),
        "AGENTS.md",
        None,
        None,
    );

    assert!(resolve(&*ctx, "AGENTS.md").await.is_none());
}

#[tokio::test]
async fn custom_file_name_is_honored() {
    let agent = TempDir::new().unwrap();
    let ctx = ctx_with(
        Some(agent.path()),
        None,
        "CUSTOM.md",
        Some("自定义文件名人格"),
        None,
    );

    let seg = resolve(&*ctx, "CUSTOM.md").await.unwrap();
    assert!(seg.body.contains("自定义文件名人格"));
    // 默认文件名不该读到内容（因为只写了 CUSTOM.md）
    let ctx2 = ctx_with(Some(agent.path()), None, "AGENTS.md", None, None);
    assert!(resolve(&*ctx2, "AGENTS.md").await.is_none());
}
