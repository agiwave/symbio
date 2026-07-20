//! registry 单元测试
//!
//! 对应源文件: `registry.rs`

use super::*;

#[test]
fn test_version_marker_file_name() {
    assert_eq!(VERSION_MARKER_FILE, ".symbio_version");
}

#[test]
fn test_version_is_semver() {
    // 简单格式校验：必须是 "X.Y.Z"
    let parts: Vec<&str> = BUILTIN_AGENTS_VERSION.split('.').collect();
    assert_eq!(parts.len(), 3, "版本号必须是 semver 格式");
    for p in parts {
        assert!(p.parse::<u32>().is_ok(), "版本号段必须是数字: {p}");
    }
}
