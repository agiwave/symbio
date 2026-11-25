//! `crates/symbio-providers/src/vdfs_service/entry.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// id 安全化：分隔符与控制字符全部落地为 `_`，点段被禁用
#[test]
fn safe_segment_neutralizes_separators() {
    assert_eq!(safe_segment("demo"), "demo");
    assert_eq!(safe_segment("  demo  "), "demo");
    assert_eq!(safe_segment("a/b\\c:d"), "a_b_c_d");
    assert_eq!(safe_segment("."), "_._");
    assert_eq!(safe_segment(".."), "_.._");
    assert_eq!(safe_segment(""), "_empty_");
    // 首尾空白清掉后仍为空 ⇒ 与空串同解
    assert_eq!(safe_segment("   "), "_empty_");
}

/// 呈现扩展名不是地址的一部分：带不带 `.skill` 必须寻到同一个条目
#[test]
fn id_of_strips_presentation_extension() {
    assert_eq!(id_of("demo", "skill"), "demo");
    assert_eq!(id_of("demo.skill", "skill"), "demo");
    assert_eq!(id_of("a/b/demo.skill", "skill"), "demo");
    // 别人的扩展名不误剥（mcp 条目不会因 `.skill` 结尾被改名）
    assert_eq!(id_of("demo.other", "skill"), "demo.other");
    // 只有 `.skill` 一个段时剥完仍回落到自身（不产生空 id）
    assert_eq!(id_of("demo", "skill"), "demo");
}

#[test]
fn pack_name_strips_zip_then_kind() {
    assert_eq!(pack_name_of("demo.zip", "skill"), "demo");
    assert_eq!(pack_name_of("demo.skill", "skill"), "demo");
    assert_eq!(pack_name_of(".zip", "skill"), ".zip");
}

/// 无名字新建的自动 id：带类别前缀、随机段定长、两次不撞、且本身就是安全段名
#[test]
fn auto_id_is_prefixed_unique_and_safe() {
    let a = auto_id("model");
    let b = auto_id("model");
    assert!(a.starts_with("model-"), "带类别前缀便于人读：{a}");
    assert_eq!(a.len(), "model-".len() + 8, "随机段定长：{a}");
    assert_ne!(a, b, "两次生成必须不同");
    assert_eq!(safe_segment(&a), a, "自动 id 必须本身就是安全段名");
}

#[test]
fn split_rel_separates_entry_and_inner_path() {
    assert_eq!(split_rel(""), None);
    assert_eq!(split_rel("/"), None);
    assert_eq!(split_rel("demo"), Some(("demo", "")));
    assert_eq!(
        split_rel("demo/scripts/run.sh"),
        Some(("demo", "scripts/run.sh"))
    );
    assert_eq!(split_rel("demo/sub/"), Some(("demo", "sub")));
}

/// 缺省节点：`ext` 由主文件名推导，不硬编码资源类型
#[test]
fn entry_default_node_derives_ext_from_manifest_file() {
    let e = Entry {
        id: "demo".into(),
        raw: Some("{}".into()),
        updated_at: Some(100),
        size: Some(2),
    };
    let n = e.node("SKILL.md");
    assert_eq!(n.ext.as_deref(), Some("md"));
    assert_eq!(n.name, "demo");
    assert_eq!(n.updated_at, Some(100));
    assert_eq!(e.node("provider.json").ext.as_deref(), Some("json"));
}

#[tokio::test]
async fn missing_category_dir_is_an_empty_list() {
    let base = std::env::temp_dir().join("symbio-entry-missing-category");
    let _ = std::fs::remove_dir_all(&base);
    assert!(list_entry_ids(&base).await.unwrap().is_empty());
}

#[tokio::test]
async fn write_then_read_then_remove_roundtrips() {
    let base = temp_base("roundtrip");
    let created = write_entry(&base, "p1", "provider.json", "{\"id\":\"p1\"}")
        .await
        .unwrap();
    assert!(created, "首次写入即新建");
    // 覆盖写不再算新建
    assert!(
        !write_entry(&base, "p1", "provider.json", "{\"id\":\"p1\",\"x\":1}")
            .await
            .unwrap()
    );

    let e = read_entry(&base, "p1", "provider.json").await.unwrap();
    assert_eq!(e.raw.as_deref(), Some("{\"id\":\"p1\",\"x\":1}"));
    assert_eq!(e.size, Some(17));
    assert!(e.updated_at.is_some());
    assert_eq!(list_entry_ids(&base).await.unwrap(), vec!["p1".to_string()]);
    assert!(entry_exists(&base, "p1"));

    remove_entry(&base, "p1").await.unwrap();
    assert!(!entry_exists(&base, "p1"));
    // 删除已不存在的条目报 NotFound（幂等与否由调用方决定）
    assert!(matches!(
        remove_entry(&base, "p1").await.unwrap_err(),
        VdfsError::NotFound(_)
    ));
    let _ = std::fs::remove_dir_all(&base);
}

/// 坏条目不得让整次列表失败：主文件缺失时降级为 `raw = None`
#[tokio::test]
async fn tolerant_read_degrades_instead_of_failing() {
    let base = temp_base("tolerant");
    std::fs::create_dir_all(base.join("broken")).unwrap();
    let e = read_entry_tolerant(&base, "broken", "SKILL.md").await;
    assert!(e.raw.is_none());
    assert_eq!(e.id, "broken");
    assert!(read_entry(&base, "broken", "SKILL.md").await.is_err());
    let _ = std::fs::remove_dir_all(&base);
}

/// base 首次创建前后、以及重建实例，都必须拿到同一把锁。
#[test]
fn mutation_lock_is_shared_across_base_creation_and_reconstruction() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("not-created-yet/model");
    let before = mutation_lock(&base);
    std::fs::create_dir_all(&base).unwrap();
    let after = mutation_lock(&base);
    let reconstructed = mutation_lock(&tmp.path().join("not-created-yet/./model"));
    assert!(Arc::ptr_eq(&before, &after));
    assert!(Arc::ptr_eq(&after, &reconstructed));
    let unrelated = mutation_lock(&tmp.path().join("other-model"));
    assert!(!Arc::ptr_eq(&after, &unrelated), "不同 base 不得共用全局锁");
}

/// 固定 `<manifest>.tmp` 是调用方文件，写入不得碰它。
#[tokio::test]
async fn write_entry_never_uses_or_cleans_the_legacy_fixed_tmp() {
    let base = temp_base("fixed-tmp");
    let dir = entry_dir(&base, "p1");
    std::fs::create_dir_all(&dir).unwrap();
    let legacy_tmp = dir.join("provider.json.tmp");
    std::fs::write(&legacy_tmp, "owned by someone else").unwrap();

    write_entry(&base, "p1", "provider.json", "new content")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&legacy_tmp).unwrap(),
        "owned by someone else"
    );
    assert_eq!(
        read_entry(&base, "p1", "provider.json")
            .await
            .unwrap()
            .raw
            .as_deref(),
        Some("new content")
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_replacement_preserves_existing_file_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = temp_base("permissions");
    write_entry(&base, "p1", "provider.json", "old")
        .await
        .unwrap();
    let path = entry_dir(&base, "p1").join("provider.json");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

    write_entry(&base, "p1", "provider.json", "new")
        .await
        .unwrap();

    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
}

#[cfg(windows)]
#[tokio::test]
async fn atomic_replacement_preserves_existing_file_acl() {
    let base = temp_base("windows-acl");
    write_entry(&base, "p1", "provider.json", "old")
        .await
        .unwrap();
    let path = entry_dir(&base, "p1").join("provider.json");
    let status = std::process::Command::new("icacls")
        .arg(&path)
        .arg("/grant:r")
        .arg("*S-1-1-0:(R)")
        .status()
        .unwrap();
    assert!(status.success());
    let acl = |path: &Path| {
        let output = std::process::Command::new("icacls")
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let before = acl(&path);

    write_entry(&base, "p1", "provider.json", "new")
        .await
        .unwrap();

    assert_eq!(acl(&path), before);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    let _ = std::fs::remove_dir_all(&base);
}

/// rename 失败时只清掉本次唯一临时文件，不留下半成品。
#[tokio::test]
async fn write_entry_cleans_its_temp_when_rename_fails() {
    let base = temp_base("rename-failure");
    let final_path = entry_dir(&base, "p1").join("provider.json");
    std::fs::create_dir_all(&final_path).unwrap();

    assert!(write_entry(&base, "p1", "provider.json", "content")
        .await
        .is_err());
    let names: Vec<_> = std::fs::read_dir(final_path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("provider.json")]);
    let _ = std::fs::remove_dir_all(&base);
}

/// 并发写只能留下一个完整 payload，且唯一临时文件全部消失。
#[tokio::test]
async fn concurrent_writes_leave_one_complete_manifest_and_no_tmp() {
    let base = temp_base("concurrent");
    let mut tasks = Vec::new();
    for i in 0..24 {
        let base = base.clone();
        let payload = format!("{i:02}:{}:end", "x".repeat(64 * 1024));
        tasks.push(tokio::spawn(async move {
            write_entry(&base, "p1", "provider.json", &payload)
                .await
                .unwrap();
            payload
        }));
    }
    let mut expected = Vec::new();
    for task in tasks {
        expected.push(task.await.unwrap());
    }

    let actual = read_entry(&base, "p1", "provider.json").await.unwrap();
    assert!(expected.contains(&actual.raw.clone().unwrap()));
    let names: Vec<_> = std::fs::read_dir(entry_dir(&base, "p1"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("provider.json")]);
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
fn try_link(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn try_link(target: &Path, link: &Path) -> bool {
    let result = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    result.is_ok()
}

#[cfg(windows)]
fn try_directory_junction(target: &Path, link: &Path) -> bool {
    use std::os::windows::process::CommandExt as _;
    let command = format!("mklink /J \"{}\" \"{}\"", link.display(), target.display());
    std::process::Command::new("cmd")
        .args(["/D", "/C"])
        .raw_arg(command)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn try_directory_link(target: &Path, link: &Path) -> bool {
    try_link(target, link) || try_directory_junction(target, link)
}

#[cfg(not(windows))]
fn try_directory_link(target: &Path, link: &Path) -> bool {
    try_link(target, link)
}

#[tokio::test]
async fn symlinked_category_root_cannot_read_or_write_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    let base = tmp.path().join("base");
    std::fs::create_dir_all(&outside).unwrap();
    if !try_directory_link(&outside, &base) {
        eprintln!("directory link creation unsupported; skipping category root test");
        return;
    }

    assert!(matches!(
        list_entry_ids(&base).await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        read_entry(&base, "p1", "provider.json").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        write_entry(&base, "p1", "provider.json", "escape").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(!outside.join("p1").exists());
}

#[tokio::test]
async fn symlinked_entry_directory_cannot_escape_the_category_root() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("base");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let dir = entry_dir(&base, "p1");
    if !try_directory_link(&outside, &dir) {
        eprintln!("directory link creation unsupported; skipping entry directory test");
        return;
    }

    assert!(!entry_exists(&base, "p1"));
    assert!(matches!(
        read_entry(&base, "p1", "provider.json").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        write_entry(&base, "p1", "provider.json", "escape").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        remove_entry(&base, "p1").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(!outside.join("provider.json").exists());
    assert!(outside.is_dir());
}

#[tokio::test]
async fn symlinked_manifest_cannot_be_read_or_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("base");
    let dir = entry_dir(&base, "p1");
    let outside = tmp.path().join("outside.json");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&outside, "keep").unwrap();
    if !try_link(&outside, &dir.join("provider.json")) {
        eprintln!("file link creation unsupported; skipping manifest test");
        return;
    }

    assert!(matches!(
        read_entry(&base, "p1", "provider.json").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        write_entry(&base, "p1", "provider.json", "overwrite").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
}

#[test]
fn temp_file_guard_cleans_owned_file_on_drop_but_not_after_disarm() {
    let tmp = tempfile::tempdir().unwrap();
    let cancelled = tmp.path().join("cancelled.tmp");
    std::fs::write(&cancelled, "partial").unwrap();
    let mut guard = TempFileGuard::new(cancelled.clone());
    guard.arm();
    drop(guard);
    assert!(!cancelled.exists());

    let renamed = tmp.path().join("renamed.tmp");
    std::fs::write(&renamed, "complete").unwrap();
    let mut guard = TempFileGuard::new(renamed.clone());
    guard.arm();
    guard.disarm();
    drop(guard);
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "complete");
}

#[tokio::test]
async fn symlinked_base_ancestor_cannot_read_write_list_or_remove() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    let parent = tmp.path().join("parent");
    std::fs::create_dir_all(&outside).unwrap();
    if !try_directory_link(&outside, &parent) {
        eprintln!("directory link creation unsupported; skipping base ancestor test");
        return;
    }
    let base = parent.join("model");
    std::fs::create_dir_all(base.join("p1")).unwrap();
    std::fs::write(base.join("p1/provider.json"), "keep").unwrap();

    assert!(matches!(
        list_entry_ids(&base).await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        read_entry(&base, "p1", "provider.json").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        write_entry(&base, "p1", "provider.json", "changed").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        remove_entry(&base, "p1").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(!entry_exists(&base, "p1"));
    assert_eq!(
        std::fs::read_to_string(base.join("p1/provider.json")).unwrap(),
        "keep"
    );
}

#[tokio::test]
async fn recursive_entry_delete_rejects_nested_reparse_before_deleting_anything() {
    let tmp = tempfile::tempdir().unwrap();
    let base = temp_base("recursive-link");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("sentinel"), "keep").unwrap();
    let dir = entry_dir(&base, "p1");
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    std::fs::write(dir.join("first.txt"), "keep too").unwrap();
    if !try_directory_link(&outside, &dir.join("nested/escape")) {
        eprintln!("directory link creation unsupported; skipping recursive delete test");
        return;
    }

    assert!(matches!(
        remove_entry(&base, "p1").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert_eq!(
        std::fs::read_to_string(dir.join("first.txt")).unwrap(),
        "keep too"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("sentinel")).unwrap(),
        "keep"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn nested_manifest_parent_symlink_is_rejected() {
    let base = temp_base("manifest-parent-link");
    let outside = temp_base("manifest-outside");
    let dir = entry_dir(&base, "p1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(outside.join("provider.json"), "keep").unwrap();
    if !try_directory_link(&outside, &dir.join("nested")) {
        eprintln!("directory link creation unsupported; skipping manifest parent test");
        return;
    }

    assert!(matches!(
        read_entry(&base, "p1", "nested/provider.json").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        write_entry(&base, "p1", "nested/provider.json", "changed").await,
        Err(VdfsError::Forbidden(_))
    ));
    assert_eq!(
        std::fs::read_to_string(outside.join("provider.json")).unwrap(),
        "keep"
    );
    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(&outside);
}

fn temp_base(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "symbio-vdfs-entry-{tag}-{}-{:?}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}
