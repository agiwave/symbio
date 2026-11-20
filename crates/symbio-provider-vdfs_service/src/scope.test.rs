use super::*;
use symbio_kernel::vdfs::{VdfsContext, VdfsRequest, VdfsResponse};

fn vctx() -> VdfsContext {
    VdfsContext::new(std::sync::Arc::new(()) as std::sync::Arc<dyn std::any::Any + Send + Sync>)
}

#[tokio::test]
async fn scoped_read_write_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let provider = ScopeDirVdfs::at(dir.path().to_path_buf());
    let ctx = vctx();

    let resp = provider
        .dispatch(
            &ctx,
            "MEMORY.md",
            VdfsRequest::Write {
                content: VdfsContent::text("hello scope"),
            },
        )
        .await
        .unwrap();
    assert!(resp.into_write().unwrap().created);

    let resp = provider
        .dispatch(&ctx, "MEMORY.md", VdfsRequest::Read)
        .await
        .unwrap();
    let content = resp.into_read().unwrap();
    assert_eq!(content.text.as_deref(), Some("hello scope"));

    // 真实文件落在根下，没有外层目录泄漏
    assert_eq!(
        std::fs::read_to_string(dir.path().join("MEMORY.md")).unwrap(),
        "hello scope"
    );
}

#[tokio::test]
async fn parent_segment_and_absolute_path_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let provider = ScopeDirVdfs::at(dir.path().to_path_buf());
    let ctx = vctx();

    let bad = provider
        .dispatch(&ctx, "../escape.md", VdfsRequest::Read)
        .await;
    assert!(bad.is_err(), "父段地址必须被拒绝");

    let bad = provider
        .dispatch(&ctx, "/etc/passwd", VdfsRequest::Read)
        .await;
    assert!(bad.is_err(), "绝对路径必须被拒绝");
}

#[tokio::test]
async fn list_and_stat_see_only_scope_entries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "agent memory").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let provider = ScopeDirVdfs::at(dir.path().to_path_buf());
    let ctx = vctx();

    let resp = provider
        .dispatch(&ctx, "", VdfsRequest::List {})
        .await
        .unwrap();
    let VdfsResponse::List(items) = resp else {
        panic!("expected list");
    };
    let names: Vec<_> = items.iter().map(|n| n.node.name.as_str()).collect();
    assert!(names.contains(&"AGENTS.md"));
    assert!(names.contains(&"sub"));

    let resp = provider
        .dispatch(&ctx, "AGENTS.md", VdfsRequest::Stat)
        .await
        .unwrap();
    let VdfsResponse::Stat(node) = resp else {
        panic!("expected stat");
    };
    assert_eq!(node.name, "AGENTS.md");
    assert_eq!(node.size, Some("agent memory".len() as u64));
}

#[test]
fn physical_root_is_controlled_exposure() {
    let dir = tempfile::tempdir().unwrap();
    let provider = ScopeDirVdfs::at(dir.path().to_path_buf());
    assert_eq!(provider.physical_root(), dir.path());
}

#[tokio::test]
async fn stat_and_list_carry_the_modified_time() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "mtime").unwrap();
    let provider = ScopeDirVdfs::at(dir.path().to_path_buf());
    let ctx = vctx();

    let stat = provider
        .dispatch(&ctx, "AGENTS.md", VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
    assert!(stat.updated_at.is_some(), "Stat 必须带 mtime，供按时间淘汰");

    let list = provider
        .dispatch(&ctx, "", VdfsRequest::List {})
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert!(
        list.iter().any(|n| n.node.updated_at.is_some()),
        "List 也要带 mtime"
    );
}

#[tokio::test]
async fn binary_files_roundtrip_as_base64() {
    let dir = tempfile::tempdir().unwrap();
    let provider = ScopeDirVdfs::at(dir.path().to_path_buf());
    let ctx = vctx();
    let bytes = vec![0u8, 1, 2, 0xff, 0xfe, 0x80];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);

    provider
        .dispatch(
            &ctx,
            "cache/index.bin",
            VdfsRequest::Write {
                content: VdfsContent::binary(encoded.clone(), bytes.len() as u64),
            },
        )
        .await
        .unwrap();

    let content = provider
        .dispatch(&ctx, "cache/index.bin", VdfsRequest::Read)
        .await
        .unwrap()
        .into_read()
        .unwrap();
    assert_eq!(
        content.b64.as_deref(),
        Some(encoded.as_str()),
        "非 UTF-8 以 b64 返回"
    );
    assert_eq!(content.size, bytes.len() as u64);
    assert!(content.text.is_none());
}

/// 开轮注入不得覆盖上层已重挂的 agent 作用域（T15：子空间会话要读自己的 AGENTS.md）
#[tokio::test]
async fn round_injection_keeps_the_existing_agent_scope() {
    use symbio_kernel::plugin::{PluginInvokeRequestExt, PluginSimpleRequest};
    use symbio_sdk::keys::{AGENT_VDFS, SESSION_VDFS, WORKSPACE_VDFS};

    let agent_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    let session_dir = tempfile::tempdir().unwrap();
    std::fs::write(agent_dir.path().join("AGENTS.md"), "评审子智能体").unwrap();

    let ctx: std::sync::Arc<dyn symbio_sdk::plugin::PluginInvokeRequest> =
        std::sync::Arc::new(PluginSimpleRequest::new(None, None));
    // 模拟 agent 插件在子树 ctx 上按 AgentScope.root 重挂的那一份
    let sub_scoped: symbio_kernel::vdfs::DynVdfsProvider =
        std::sync::Arc::new(ScopeDirVdfs::at(agent_dir.path().to_path_buf()));
    ctx.set(AGENT_VDFS, Some(sub_scoped));

    attach_round_providers(&*ctx, Some(work_dir.path()), Some(session_dir.path()));

    // agent 层仍是子智能体目录，而不是被 agent_home_root() 顶回 {}
    let kept = ctx.get(AGENT_VDFS).flatten().unwrap();
    let read = kept
        .dispatch(&vctx(), "AGENTS.md", VdfsRequest::Read)
        .await
        .unwrap()
        .into_read()
        .unwrap();
    assert_eq!(read.text.as_deref(), Some("评审子智能体"));

    // 另外两层按本次开轮参数重挂
    assert!(ctx.get(WORKSPACE_VDFS).flatten().is_some());
    assert!(ctx.get(SESSION_VDFS).flatten().is_some());
}
