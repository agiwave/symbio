//! path.rs 单元测试
//!
//! 对应源文件: `path.rs`

use super::*;

#[test]
fn test_resolve_workspace_dir_basic() {
    #[cfg(unix)]
    let test_path = "/tmp/proj";
    #[cfg(windows)]
    let test_path = "C:\\tmp\\proj";

    let p = resolve_workspace_dir(Some(test_path)).unwrap();
    #[cfg(unix)]
    assert_eq!(p, PathBuf::from("/tmp/proj/.symbio/agents"));
    #[cfg(windows)]
    assert_eq!(p, PathBuf::from("C:\\tmp\\proj\\.symbio\\agents"));
}

#[test]
fn test_resolve_workspace_dir_rejects_relative() {
    assert!(resolve_workspace_dir(Some("proj")).is_none());
    assert!(resolve_workspace_dir(Some("./proj")).is_none());
}

#[test]
fn test_resolve_workspace_dir_rejects_parent_escape() {
    // `..` 在中间会被拒绝
    assert!(resolve_workspace_dir(Some("/tmp/../escape")).is_none());
}

#[test]
fn test_resolve_workspace_dir_none() {
    assert!(resolve_workspace_dir(None).is_none());
}

#[test]
fn test_resolve_workspace_dir_tilde() {
    // 展开后若是绝对路径（取决于平台）则通过
    let p = resolve_workspace_dir(Some("~/myproj"));
    // macOS/Linux 期望展开为 /Users/x/myproj
    if cfg!(unix) {
        assert!(p.is_some());
        let p = p.unwrap();
        assert!(p.ends_with("myproj/.symbio/agents"));
    }
}

#[test]
fn test_global_agents_dir_has_symbio() {
    let p = resolve_global_agents_dir();
    assert!(p.ends_with(".symbio/plugins/agent"));
}

#[test]
fn test_safe_join_basic() {
    let base = Path::new("/tmp/base");
    let p = safe_join(base, "sub/file.yaml").unwrap();
    assert_eq!(p, PathBuf::from("/tmp/base/sub/file.yaml"));
}

#[test]
fn test_safe_join_rejects_parent_escape() {
    let base = Path::new("/tmp/base");
    assert!(safe_join(base, "../escape").is_none());
    assert!(safe_join(base, "sub/../../escape").is_none());
}

#[test]
fn test_safe_join_rejects_empty() {
    let base = Path::new("/tmp/base");
    assert!(safe_join(base, "").is_none());
}

#[test]
fn test_normalize_path() {
    assert_eq!(
        normalize_path(Path::new("/a/b/../c")),
        PathBuf::from("/a/c")
    );
    assert_eq!(normalize_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
}
