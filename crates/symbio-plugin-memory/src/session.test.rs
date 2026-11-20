//! `session.rs` 的单元测试 —— 本层的个性（落位 / 挂载名 / 闸门注入）。

use super::*;
use std::path::Path;
use symbio_providers::MemoryFile;
use tempfile::TempDir;

/// 会话记忆的物理落位（测试断言用；生产路径由 provider 持有根）
fn memory_path(session_dir: &str) -> std::path::PathBuf {
    Path::new(session_dir).join(SESSION_MEMORY_FILE)
}

/// 测试专用的文件后端构造（保留落位规则用例）。
///
/// 空白目录字符串 = 无会话作用域，与生产侧 provider 缺失同一口径。
fn store_at(
    session_dir: Option<&str>,
    write_max_bytes: usize,
    inject_max_bytes: usize,
) -> MemoryFile {
    match session_dir.filter(|d| !d.trim().is_empty()) {
        Some(dir) => MemoryFile::new(Some(memory_path(dir)), write_max_bytes, inject_max_bytes),
        None => MemoryFile::absent(write_max_bytes, inject_max_bytes),
    }
}

#[tokio::test]
async fn memory_lives_in_the_supplied_session_directory() {
    let path = memory_path("/symbio-test/session/abc");
    assert_eq!(path.file_name().unwrap(), SESSION_MEMORY_FILE);
    assert_eq!(
        path.parent().unwrap(),
        Path::new("/symbio-test/session/abc")
    );
}

#[tokio::test]
async fn mount_name_identifies_the_scope() {
    assert_eq!(rel_path(), "SESSION.md");
    let spec = node_spec();
    assert_eq!(spec.name, Some("SESSION.md"));
    assert_eq!(spec.kind, "session");
}

#[tokio::test]
async fn missing_session_dir_means_no_scope() {
    let m = store_at(None, 1024, 256);
    assert!(!m.has_scope());
    assert!(m.path().is_none());
    assert_eq!(m.inject().await.unwrap(), None);

    for blank in ["", "   "] {
        assert!(!store_at(Some(blank), 1024, 256).has_scope());
    }
}

#[tokio::test]
async fn scope_carries_the_two_gates_into_the_shared_impl() {
    let m = store_at(Some("/symbio-test/session/abc"), 1234, 321);
    assert!(m.has_scope());
    assert_eq!(
        m.path(),
        Some(memory_path("/symbio-test/session/abc").as_path())
    );
    assert_eq!(m.write_max_bytes(), 1234);
    assert_eq!(m.inject_max_bytes(), 321);
    assert_eq!(m.file_name(), Some(SESSION_MEMORY_FILE));
}

#[tokio::test]
async fn segment_spec_names_all_three_scopes() {
    let s = segment_spec("@vfs/memory/SESSION.md");
    assert_eq!(s.title, "会话记忆");
    assert_eq!(s.address, "@vfs/memory/SESSION.md");
    assert!(s
        .note
        .unwrap_or_default()
        .contains("与【工作区记忆】【智能体记忆】相互独立"));
    assert!(s.empty_hint.contains("暂无内容"));
}

#[tokio::test]
async fn segment_round_trips_through_a_real_file() {
    let tmp = TempDir::new().unwrap();
    let m = MemoryFile::new(Some(tmp.path().join(SESSION_MEMORY_FILE)), 1024, 256);
    m.write("本会话约定：所有时间用 UTC。").await.unwrap();

    let seg = m
        .segment(&segment_spec("@vfs/memory/SESSION.md"))
        .await
        .unwrap()
        .unwrap();
    assert!(seg.contains("【会话记忆】"), "{seg}");
    assert!(seg.contains("@vfs/memory/SESSION.md"), "{seg}");
    assert!(seg.contains("1024"), "{seg}");
    assert!(seg.contains("本会话约定：所有时间用 UTC。"), "{seg}");
}
