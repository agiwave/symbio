//! `agent/host/memory.rs` 的单元测试 —— 智能体记忆的**落位、作用域与片段口径**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 记忆的**机制**（读写 / 两道闸门 / 排版 / 节点形状）由 `symbio_core::memory`
//! 自己测；这里钉的是**本层的个性**：落在哪、地址是什么、bundle 不存在时怎么降级、
//! 以及「与工作区记忆同名不同域」有没有说清楚。

use super::super::config::AgentConfig;
use super::*;
use crate::symbio_core::VdfsAccess;
use tempfile::TempDir;

/// 在工作区级落一个最小 bundle（不经 zip：以下用例只关心记忆的落位与作用域）
fn workspace_with_bundle() -> (TempDir, BundleStore) {
    let dir = TempDir::new().unwrap();
    let store = BundleStore::new(
        dir.path().join("global-agent"),
        Some(dir.path().to_str().unwrap()),
    );
    let bundle = dir.path().join(".symbio/agent/b");
    std::fs::create_dir_all(bundle.join("prompts")).unwrap();
    std::fs::write(
        bundle.join("manifest.yaml"),
        "spec: \"oab/v1\"\nid: \"b\"\nname: \"B\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^1\"\n",
    )
    .unwrap();
    assert!(store.get("b").is_some(), "前置：bundle 应被扫描到");
    (dir, store)
}

/// 按插件**默认配置**构造记忆门面（闸门取值从 `AgentConfig` 读，不写第二份字面量）
fn store_of(bundles: &BundleStore, id: &str) -> MemoryFile {
    let cfg = AgentConfig::default();
    store(
        bundles,
        id,
        cfg.effective_memory_max_bytes(),
        cfg.effective_memory_inject_bytes(),
    )
}

// ==================== 落位与地址 ====================

/// 记忆落在 **bundle 自己的目录**里（不是工作区目录）
#[test]
fn memory_address_is_inside_the_bundle_dir() {
    assert_eq!(
        memory_address("com.acme.cr"),
        ".vdfs/agent/com.acme.cr/AGENTS.md"
    );
}

#[test]
fn memory_lives_next_to_the_bundle_manifest() {
    let (dir, bundles) = workspace_with_bundle();
    let m = store_of(&bundles, "b");

    assert!(m.has_scope());
    assert_eq!(
        m.path().unwrap(),
        dir.path().join(".symbio/agent/b/AGENTS.md")
    );
    assert_eq!(m.file_name(), AGENTS_FILE, "节点名 = 真实文件名");
    // 不是工作区根的那个 AGENTS.md —— 那是 work 插件的作用域
    assert_ne!(m.path().unwrap(), dir.path().join(AGENTS_FILE));
}

/// bundle 不存在 = **无作用域**，是正常状态而不是崩溃
#[test]
fn missing_bundle_means_no_scope() {
    let (_dir, bundles) = workspace_with_bundle();
    let m = store_of(&bundles, "nope");

    assert!(!m.has_scope());
    assert!(m.read().is_err());
    assert!(m.write("x").is_err());
    // 注入是 None —— 静默跳过，不往收集期错误桶里塞东西
    assert_eq!(m.inject().unwrap(), None);
    assert_eq!(
        m.segment(&segment_spec(&memory_address("nope"))).unwrap(),
        None
    );
}

// ==================== 端到端：写 → 读 → 片段 ====================

#[test]
fn memory_roundtrips_and_renders_the_kernel_segment() {
    let (_dir, bundles) = workspace_with_bundle();
    let m = store_of(&bundles, "b");

    // 还没有记忆 → 空串（不是错误）
    assert_eq!(m.read().unwrap(), "");

    let text = "该智能体记住：先写测试。";
    m.write(text).unwrap();
    assert_eq!(m.read().unwrap(), text);

    let seg = m
        .segment(&segment_spec(&memory_address("b")))
        .unwrap()
        .unwrap();
    assert!(seg.contains("【智能体记忆】"), "{seg}");
    assert!(seg.contains(".vdfs/agent/b/AGENTS.md"), "{seg}");
    assert!(seg.contains(text), "正文要原样带上: {seg}");
    assert!(
        seg.contains("vdfs_read") && seg.contains("vdfs_write"),
        "{seg}"
    );
    assert!(
        seg.contains("与【工作区记忆】相互独立"),
        "两层记忆同名不同域，不点明模型会写错地方: {seg}"
    );
}

#[test]
fn empty_memory_teaches_how_to_remember() {
    let (_dir, bundles) = workspace_with_bundle();
    let seg = store_of(&bundles, "b")
        .segment(&segment_spec(&memory_address("b")))
        .unwrap()
        .unwrap();

    assert!(seg.contains("暂无记忆"), "{seg}");
    assert!(seg.contains("vdfs_write"));
    assert!(!seg.contains("已截断"), "空记忆不得谎报截断");
}

// ==================== 两道闸门由内核执行（本层不重复实现） ====================

#[test]
fn write_gate_is_enforced_by_the_kernel() {
    let (_dir, bundles) = workspace_with_bundle();
    let m = store(&bundles, "b", 4, 256);

    let err = m.write(&"x".repeat(8)).unwrap_err();
    assert!(err.contains("超出容量上限"), "{err}");
    assert_eq!(m.read().unwrap(), "", "被拒绝的写入不得留下半截内容");
}

#[test]
fn inject_gate_truncates_and_points_at_the_address() {
    let (_dir, bundles) = workspace_with_bundle();
    let m = store(&bundles, "b", 256, 4);
    m.write("0123456789").unwrap();

    let seg = m
        .segment(&segment_spec(&memory_address("b")))
        .unwrap()
        .unwrap();
    assert!(seg.contains("已截断"), "{seg}");
    assert!(!seg.contains("0123456789"), "超预算的部分不注入: {seg}");
    assert!(
        seg.contains(".vdfs/agent/b/AGENTS.md"),
        "要指路读全文: {seg}"
    );
}

// ==================== VDFS 节点 ====================

/// 节点形状由内核决定 —— 与 work / session 两层同源，不在这里手搓一份
#[test]
fn node_shape_comes_from_the_kernel() {
    let (_dir, bundles) = workspace_with_bundle();
    let m = store_of(&bundles, "b");
    m.write("内容").unwrap();

    let n = m.node(&node_spec());
    assert_eq!(n.name, AGENTS_FILE, "节点名 = 真实文件名");
    assert_eq!(n.title, SEGMENT_TITLE);
    assert_eq!(
        n.kind, PLUGIN_AGENT,
        "场景标签用所属插件，不另造一个没有消费者的 `memory`"
    );
    assert_eq!(n.size, Some("内容".len() as u64));
    assert!(n.updated_at.is_some(), "节点要用它做排序");
    assert_eq!(n.access, VdfsAccess::READ_WRITE, "记忆恒可读写");
    // 缺省 ext 由文件名字面扩展名推导（`AGENTS.md` → `md`），渲染器据此分发
    assert_eq!(n.ext, None, "不显式声明，交给机制推导");
}
