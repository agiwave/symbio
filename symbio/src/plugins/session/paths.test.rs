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
    assert_eq!(safe_id("sess_1717_a1b2"), "sess_1717_a1b2");
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
            crate::providers::vdfs_safe_segment(id),
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
    let dir = session_subdir(
        std::path::Path::new("/symbio-test/session"),
        "a/b",
        TOOL_ARCHIVES_SUBDIR,
    );
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

/// 具名新建：**地址末段即会话 id**；无名目标（挂载根 / 空末段）⇒ `None`（provider 生成）
///
/// 与 `safe_id` 的方向相反，但同属一套寻址规则（见本模块头）。
#[test]
fn session_id_from_new_path_uses_the_address_as_identity() {
    assert_eq!(
        session_id_from_new_path("我的会话").as_deref(),
        Some("我的会话")
    );
    assert_eq!(
        session_id_from_new_path("dir/我的会话").as_deref(),
        Some("我的会话")
    );
    // 不剥扩展名：会话寻址里 `.session` 不是地址的一部分（见函数文档）
    assert_eq!(
        session_id_from_new_path("abc.session").as_deref(),
        Some("abc.session")
    );
    // 无名目标 ⇒ 交给 provider 生成
    assert_eq!(session_id_from_new_path(""), None);
    assert_eq!(session_id_from_new_path("/"), None);
    assert_eq!(session_id_from_new_path("dir/"), None);
    assert_eq!(session_id_from_new_path("   "), None);
}

/// 全会话事实源枚举（部署级读侧的遍历入口）：顶层 + **嵌套子会话**（任意深度），
/// 非事实源子树（`tool_archives` 等）天然被过滤——判据是存在性，不是排除名单。
///
/// 只扫一层的读方会整棵子树都看不见：淘汰闸的覆盖层 ⇒「换个**子会话**绕开淘汰」、
/// stats 实例聚合 ⇒ 文档说着「所有会话」而实现少数一层。本测试钉住范围这一半。
#[test]
fn session_wals_deep_covers_nested_sub_sessions() {
    let root = std::env::temp_dir().join(format!("symbio-paths-deep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("建存储根");

    // 布局：两个顶层会话，其中一个带子会话，子会话再带子会话（分形）；
    // 另加 `tool_archives` 子目录（非事实源）。
    let mk = |rel: &str| {
        let dir = root.join(rel);
        std::fs::create_dir_all(&dir).expect("建目录");
        std::fs::write(dir.join(V2_WAL_FILE), b"").expect("建 WAL 占位");
        dir
    };
    let top_a = mk("sess-a");
    let top_b = mk("sess-b");
    let child = mk("sess-a/sessions/child-1");
    let grandchild = mk("sess-a/sessions/child-1/sessions/grand-1");
    std::fs::create_dir_all(top_a.join(TOOL_ARCHIVES_SUBDIR)).ok();

    let mut wals = session_wals_deep(&root);
    wals.sort();
    let mut expected = vec![
        top_a.join(V2_WAL_FILE),
        top_b.join(V2_WAL_FILE),
        child.join(V2_WAL_FILE),
        grandchild.join(V2_WAL_FILE),
    ];
    expected.sort();
    assert_eq!(
        wals, expected,
        "全会话事实源 = 顶层 + 嵌套子会话（任意深度）；tool_archives 无 WAL ⇒ 不计"
    );

    // 不存在的根 ⇒ 空（读方自己决定这是「没有会话」还是「目录错了」）。
    assert!(session_wals_deep(&root.join("nope")).is_empty());

    std::fs::remove_dir_all(&root).ok();
}
