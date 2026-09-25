//! `symbio/src/plugins/agent/host/store.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn strip_manifest_root_accepts_both_layouts() {
    // 根目录布局
    assert_eq!(
        AgentDirStore::strip_manifest_root("providers/persona/provider.yaml", "com.acme.cr"),
        Some("providers/persona/provider.yaml".into())
    );
    // 单顶层目录布局
    assert_eq!(
        AgentDirStore::strip_manifest_root("com.acme.cr/manifest.yaml", "com.acme.cr"),
        Some("manifest.yaml".into())
    );
    // 顶层目录名与 agent_id 无关的杂项文件：原样接受（根目录布局语义；
    // import 只解析 manifest.yaml，无关文件落盘无害）
    assert_eq!(
        AgentDirStore::strip_manifest_root("other/manifest.yaml", "com.acme.cr"),
        Some("other/manifest.yaml".into())
    );
    // 目录项 → 跳过
    assert_eq!(
        AgentDirStore::strip_manifest_root("com.acme.cr/providers/", "com.acme.cr"),
        None
    );
}

#[test]
fn absolutize_rejects_traversal_segments() {
    let base = Path::new("/tmp/agent_dirs/com.acme");
    let p = absolutize(base, "providers/../../etc/passwd");
    // `..` 段被丢弃，路径仍锁定在 base 内
    assert!(p.starts_with(base));
    assert!(p.to_string_lossy().contains("etc"));
}

#[test]
fn path_sandbox_accepts_any_inner_path() {
    // v2 不再解释布局：只挡住逃逸，其它一律放行（含 manifest.yaml 等）
    for ok in [
        "manifest.yaml",
        "skill/foo/SKILL.md",
        "mcp/x/server.json",
        "assets/a.png",
    ] {
        assert_eq!(normalize_item_path(ok).as_deref(), Ok(ok));
    }
    // 反斜杠 + ./ 前缀归一化
    assert_eq!(
        normalize_item_path("./skill\\x/SKILL.md").as_deref(),
        Ok("skill/x/SKILL.md")
    );
}

#[test]
fn path_sandbox_rejects_escapes() {
    for bad in [
        "../etc/passwd",
        "/abs/path.md",
        "a/../../b",
        "a//b",
        "a/",
        "",
    ] {
        assert!(normalize_item_path(bad).is_err(), "should reject `{bad}`");
    }
}

/// 在本插件目录落一个最小 agent 目录（不经 zip：以下用例只关心写入闸门）
fn workspace_store_with_agent_dir() -> (tempfile::TempDir, AgentDirStore) {
    let dir = tempfile::TempDir::new().unwrap();
    let store = AgentDirStore::new(dir.path().join("global-agent"));
    let agent_dir = dir.path().join("global-agent").join("b");
    std::fs::create_dir_all(agent_dir.join("prompts")).unwrap();
    std::fs::write(
        agent_dir.join("manifest.yaml"),
        "spec: \"oab/v1\"\nid: \"b\"\nname: \"B\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^1\"\n",
    )
    .unwrap();
    assert!(store.get("b").is_some(), "前置：agent 目录应被扫描到");
    (dir, store)
}

/// 写入闸门：超限**拒绝**，且不留下半截内容
#[test]
fn write_item_rejects_oversized_content_without_touching_the_file() {
    let (_dir, store) = workspace_store_with_agent_dir();

    store
        .write_item("b", "prompts/persona.md", "0123456789", 10)
        .unwrap();
    let err = store
        .write_item("b", "prompts/persona.md", "0123456789X", 10)
        .unwrap_err();
    assert!(err.contains("超出容量上限"), "{err}");
    assert!(err.contains("10"), "错误信息要带上限值：{err}");
    assert_eq!(
        store.read_item("b", "prompts/persona.md").unwrap(),
        "0123456789",
        "被拒绝的写入不得改动文件"
    );
}

/// 智能体记忆**落位**在 agent 目录自己的目录（不是工作区目录），文件名与工作区级同名。
///
/// 读写与两道容量闸门不在这里测——它们已收口到内核，用例在
/// `agent/host/memory.test.rs`（本模块只回答「记忆文件在哪」）。
#[test]
fn memory_lives_in_agent_dir() {
    let (dir, store) = workspace_store_with_agent_dir();
    let path = store.memory_path("b").unwrap();
    assert_eq!(
        path,
        dir.path().join("global-agent/b").join(MEMORY_AGENTS_FILE)
    );
    assert_eq!(path.file_name().unwrap(), "AGENTS.md");
    // 不是工作区根的那个 AGENTS.md
    assert_ne!(path, dir.path().join(MEMORY_AGENTS_FILE));
    // agent 目录不存在 → 明确报错（`memory::store` 据此构造「无作用域」门面）
    assert!(store.memory_path("nope").is_err());
}
