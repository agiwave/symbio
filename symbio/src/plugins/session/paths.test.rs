//! `paths` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `paths.rs` 只保留生产代码，测试全部放本文件。

use super::*;

#[test]
fn safe_id_replaces_path_separators_and_colon() {
    assert_eq!(safe_id("a/b"), "a_b");
    assert_eq!(safe_id("a\\b"), "a_b");
    assert_eq!(safe_id("c:tmp"), "c_tmp");
    assert_eq!(safe_id("a/b\\c:d"), "a_b_c_d");
}

#[test]
fn safe_id_keeps_normal_ids_untouched() {
    assert_eq!(safe_id("plain-id_123"), "plain-id_123");
    assert_eq!(safe_id("v2_sess_1717_a1b2"), "v2_sess_1717_a1b2");
    // 空 id 归一为占位段名（与资源条目同一规则）；空 / `_t_` 前缀的会话
    // 走不落盘的临时存储，因此这条映射只是防御性一致，不影响任何磁盘路径。
    assert_eq!(safe_id(""), "_empty_");
}

/// 会话目录名与 VDFS 资源条目目录名**同一份规则**（本次收敛的不变量）：
/// 过去两处各写一版，`*` `?` `.` `..` 等字符的处置不一致，同一个 id 在
/// 会话侧与资源侧会落到不同段名下。
#[test]
fn safe_id_is_the_same_rule_as_vdfs_entry_segments() {
    for id in [
        "a/b",
        "a\\b",
        "c:tmp",
        "plain-id_123",
        "..",
        ".",
        "weird*name?.md",
        " padded ",
    ] {
        assert_eq!(
            safe_id(id),
            crate::providers::vdfs_service::entry::safe_segment(id),
            "id `{id}` 的会话段名与资源段名必须同解"
        );
    }
}

#[test]
fn retrieval_hint_mentions_unified_entry() {
    assert!(!RETRIEVAL_HINT.is_empty());
    assert!(RETRIEVAL_HINT.contains("vdfs_read"));
    assert!(RETRIEVAL_HINT.contains("offset/limit"));
}

#[test]
fn session_subdir_appends_under_safe_session_dir() {
    let dir = session_subdir("a/b", TOOL_ARCHIVES_SUBDIR);
    let parts: Vec<_> = dir.components().collect();
    // 末两段应为 safe_id 段与子目录段
    assert!(parts.len() >= 2);
    let last_two: Vec<String> = parts[parts.len() - 2..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        last_two,
        vec!["a_b".to_string(), "tool_archives".to_string()]
    );
}
