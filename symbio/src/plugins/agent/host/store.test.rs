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

/// 挂载根清单只含**通过 §10 校验**的目录：解析得了但过不了门槛的不列。
#[test]
fn list_only_returns_dirs_passing_the_access_gate() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("global-agent");
    let store = AgentDirStore::new(root.clone());
    for (id, manifest) in [
        (
            "good",
            "spec: \"agent-dir/v2\"\nid: \"good\"\nname: \"G\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n",
        ),
        (
            "old",
            "spec: \"oab/v1\"\nid: \"old\"\nname: \"O\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^1\"\n",
        ),
        (
            "noreq",
            "spec: \"agent-dir/v2\"\nid: \"noreq\"\nname: \"N\"\nversion: \"1.0.0\"\n",
        ),
    ] {
        std::fs::create_dir_all(root.join(id)).unwrap();
        std::fs::write(root.join(id).join("manifest.yaml"), manifest).unwrap();
    }
    let ids: Vec<String> = store
        .list()
        .into_iter()
        .map(|r| r.manifest.id.clone())
        .collect();
    assert_eq!(ids, vec!["good"], "只列通过门槛者，实际：{ids:?}");
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
        "spec: \"agent-dir/v2\"\nid: \"b\"\nname: \"B\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n",
    )
    .unwrap();
    assert!(store.get("b").is_some(), "前置：agent 目录应被扫描到");
    (dir, store)
}

/// 智能体记忆**落位**在 agent 目录自己的目录（不是工作区目录），文件名与工作区级同名。
///
/// 读写与两道容量闸门不在这里测——它们已收口到共享实现，用例在
/// `agent/host/memory.test.rs`（本模块只回答「记忆文件在哪」）。
#[test]
fn memory_lives_in_agent_dir() {
    let (dir, store) = workspace_store_with_agent_dir();
    let path = store.memory_path("b").unwrap();
    assert_eq!(
        path,
        dir.path().join("global-agent/b").join(AGENT_MEMORY_FILE)
    );
    assert_eq!(path.file_name().unwrap(), "AGENTS.md");
    // 不是工作区根的那个 AGENTS.md
    assert_ne!(path, dir.path().join(AGENT_MEMORY_FILE));
    // agent 目录不存在 → 明确报错（`memory::store` 据此构造「无作用域」门面）
    assert!(store.memory_path("nope").is_err());
}
