//! `crates/symbio-provider-vdfs_service/src/pack.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 构造内存 zip（按给定顺序写入条目；`None` 内容 = 只建目录条目）
fn make_zip(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            match content {
                Some(bytes) => {
                    w.start_file(*name, opts).unwrap();
                    w.write_all(bytes).unwrap();
                }
                None => {
                    w.add_directory(*name, opts).unwrap();
                }
            }
        }
        w.finish().unwrap();
    }
    buf
}

/// 解包：跳过目录 / 隐藏文件 / `__MACOSX`，并规范化前导 `./` 与 `/`
#[test]
fn parse_pack_skips_dirs_and_metadata() {
    let bytes = make_zip(&[
        ("SKILL.md", Some(b"# demo")),
        ("scripts/run.sh", Some(b"echo hi")),
        ("scripts/", None),
        ("__MACOSX/._SKILL.md", Some(b"junk")),
        (".hidden", Some(b"junk")),
        ("./nested/ok.txt", Some(b"ok")),
    ]);
    let out = parse_pack(&bytes).unwrap();
    let names: Vec<&str> = out.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        names,
        vec!["SKILL.md", "scripts/run.sh", "nested/ok.txt"],
        "目录条目与元数据不落盘：`{names:?}`"
    );
    assert_eq!(out[0].1, b"# demo".to_vec());
}

/// zip-slip 与 Windows 绝对路径必须在解析阶段显式拒绝，不能静默跳过。
#[test]
fn parse_pack_never_yields_an_escaping_entry() {
    for hostile in [
        "evil/../../escape.txt",
        "../../escape.txt",
        "a/../../../escape.txt",
        "..\\..\\escape.txt",
        "/absolute.txt",
        "C:/absolute.txt",
        "C:drive-relative.txt",
        "//server/share/escape.txt",
        "safe/file:stream.txt",
    ] {
        let bytes = make_zip(&[(hostile, Some(b"boom"))]);
        assert!(parse_pack(&bytes).is_err(), "条目 `{hostile}` 必须被拒绝");
    }
}

/// 落盘侧的同一性质：解包后目标目录之外不得出现任何文件
#[tokio::test]
async fn extract_pack_confines_writes_to_the_entry_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let entry_dir = tmp.path().join("demo");
    let bytes = make_zip(&[
        ("SKILL.md", Some(b"# ok")),
        ("evil/../../escape.txt", Some(b"boom")),
    ]);
    assert!(extract_pack(&entry_dir, &bytes).await.is_err());
    assert!(!entry_dir.exists(), "解析失败不得创建目标目录");
    assert!(
        !tmp.path().join("escape.txt").exists(),
        "包内 `..` 条目不得写到条目目录之外"
    );
}

/// 单顶层目录的打包习惯：剥离该层，内容平铺到条目目录
#[test]
fn strip_common_root_flattens_single_root() {
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        ("pkg/SKILL.md".into(), b"a".to_vec()),
        ("pkg/scripts/s.sh".into(), b"b".to_vec()),
    ];
    strip_common_root(&mut entries);
    let names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(names, vec!["SKILL.md", "scripts/s.sh"]);

    // 不共享顶层目录 ⇒ 原样保留（避免误伤多根包）
    let mut mixed: Vec<(String, Vec<u8>)> =
        vec![("a/x.md".into(), Vec::new()), ("b/y.md".into(), Vec::new())];
    strip_common_root(&mut mixed);
    let names: Vec<&str> = mixed.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(names, vec!["a/x.md", "b/y.md"]);
}

/// 非法输入：非 base64 / 非 zip，都转为可读错误（不 panic）
#[test]
fn pack_errors_are_reported() {
    assert!(decode_b64("!!!not-base64!!!").is_err());
    assert!(parse_pack(b"not a zip at all").is_err());
    let empty = make_zip(&[("only-dir/", None)]);
    assert!(parse_pack(&empty).unwrap().is_empty());
}

/// 打包：单顶层目录 = 条目 id，跳过隐藏文件——与导入端恰好配对
#[test]
fn zip_dir_roundtrips_with_extract() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo");
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(dir.join("SKILL.md"), b"# demo").unwrap();
    std::fs::write(dir.join("scripts").join("run.sh"), b"echo hi").unwrap();
    std::fs::write(dir.join(".DS_Store"), b"junk").unwrap();

    let bytes = zip_dir(&dir, "demo").unwrap();
    let mut entries = parse_pack(&bytes).unwrap();
    strip_common_root(&mut entries);
    let mut names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["SKILL.md", "scripts/run.sh"],
        "导出 → 导入可原样往返：`{names:?}`"
    );
}

/// 解包即整目录覆盖：旧文件不得残留在新包里
#[tokio::test]
async fn extract_pack_replaces_whole_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo");
    std::fs::create_dir_all(dir.join("stale")).unwrap();
    std::fs::write(dir.join("stale/old.txt"), b"old").unwrap();

    let bytes = make_zip(&[("SKILL.md", Some(b"# new"))]);
    let n = extract_pack(&dir, &bytes).await.unwrap();
    assert_eq!(n, 1);
    assert!(dir.join("SKILL.md").exists());
    assert!(
        !dir.join("stale/old.txt").exists(),
        "导入即替换：旧目录里未被包覆盖的文件必须消失"
    );
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("demo")]);
}

/// 载荷形状：`filename` + `b64` 就是前端认得的全部（它不认识「导出」）
#[test]
fn pack_payload_is_a_file_shape() {
    let p = VdfsPack::new("demo", b"hi");
    assert_eq!(p.filename, "demo.zip");
    assert_eq!(decode_b64(&p.b64).unwrap(), b"hi".to_vec());
}

/// **出向与入向同形**：导出的包原样装进导入载荷，字段一个不多一个不少。
///
/// 这条是「导出 / 导入是一对逆向动作」在**载荷层**的落点：若哪天给其中一侧
/// 加了字段（如 `id`），本用例会红——那意味着往返契约被单方面改了。
#[test]
fn unpack_mirrors_pack() {
    let pack = VdfsPack::new("demo", b"hi");
    let value = serde_json::to_value(&pack).unwrap();
    // 去掉出向独有的 `id`（导入的 id 由 provider 决定）即是入向载荷
    let mut obj = value.as_object().unwrap().clone();
    obj.remove("id");
    let unpack = VdfsUnpack::from_payload(Some(&serde_json::Value::Object(obj))).unwrap();
    assert_eq!(unpack.filename, "demo.zip");
    assert_eq!(unpack.bytes().unwrap(), b"hi".to_vec());
    assert_eq!(unpack.name_of("demo-kind"), "demo");
}

/// 缺载荷 / 缺字段 / 非对象：都转为可读错误，不 panic
#[test]
fn pack_rejects_too_many_entries_and_oversized_files() {
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        for i in 0..=MAX_PACK_ENTRIES {
            writer.start_file(format!("{i}.txt"), options).unwrap();
        }
        writer.finish().unwrap();
    }
    assert!(parse_pack(&buf).unwrap_err().0.contains("条目过多"));

    let bytes = make_zip(&[("large.bin", Some(&vec![0; MAX_PACK_ENTRY_SIZE + 1]))]);
    assert!(parse_pack(&bytes).unwrap_err().0.contains("条目过大"));
}

#[tokio::test]
async fn pack_rejects_duplicate_and_file_directory_collisions() {
    for entries in [
        vec![
            ("a.txt", Some(b"a".as_slice())),
            ("./a.txt", Some(b"b".as_slice())),
        ],
        vec![
            ("a", Some(b"a".as_slice())),
            ("a/b.txt", Some(b"b".as_slice())),
        ],
        vec![
            ("a/b.txt", Some(b"b".as_slice())),
            ("a", Some(b"a".as_slice())),
        ],
        vec![
            ("a", Some(b"a".as_slice())),
            ("a-foo", Some(b"other".as_slice())),
            ("a/b.txt", Some(b"b".as_slice())),
        ],
        vec![
            ("a/b.txt", Some(b"b".as_slice())),
            ("a-foo", Some(b"other".as_slice())),
            ("a", Some(b"a".as_slice())),
        ],
    ] {
        let bytes = make_zip(&entries);
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("demo");
        assert!(extract_pack(&target, &bytes).await.is_err());
        assert!(!target.exists(), "冲突包失败时不得留下目标目录");
    }
}

#[test]
fn publish_guard_restores_the_old_directory_after_a_failed_publish() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("demo");
    let staging = tmp.path().join("staging");
    let backup = tmp.path().join("backup");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("new.txt"), "new").unwrap();
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join("old.txt"), "old").unwrap();

    drop(PackPublishGuard {
        target: target.clone(),
        staging,
        backup,
        old_moved: true,
        published: false,
    });

    assert_eq!(
        std::fs::read_to_string(target.join("old.txt")).unwrap(),
        "old"
    );
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("demo")]);
}

#[test]
fn publish_guard_keeps_the_only_old_copy_when_rollback_cannot_replace_target() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("demo");
    let staging = tmp.path().join("staging");
    let backup = tmp.path().join("backup");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("other.txt"), "other").unwrap();
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join("old.txt"), "old").unwrap();

    drop(PackPublishGuard {
        target: target.clone(),
        staging: staging.clone(),
        backup: backup.clone(),
        old_moved: true,
        published: false,
    });

    assert_eq!(
        std::fs::read_to_string(target.join("other.txt")).unwrap(),
        "other"
    );
    assert_eq!(
        std::fs::read_to_string(backup.join("old.txt")).unwrap(),
        "old"
    );
    assert!(!staging.exists());
}

#[test]
fn published_guard_cleans_the_backup_without_rolling_back() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("demo");
    let staging = tmp.path().join("staging");
    let backup = tmp.path().join("backup");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("new.txt"), "new").unwrap();
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join("old.txt"), "old").unwrap();

    drop(PackPublishGuard {
        target: target.clone(),
        staging,
        backup: backup.clone(),
        old_moved: true,
        published: true,
    });

    assert_eq!(
        std::fs::read_to_string(target.join("new.txt")).unwrap(),
        "new"
    );
    assert!(!backup.exists());
}

#[test]
fn zip_export_rejects_links_instead_of_copying_their_targets() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo");
    let outside = tmp.path().join("secret.txt");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&outside, "secret").unwrap();

    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, dir.join("link.txt")).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, dir.join("link.txt")).is_ok();
    if !linked {
        eprintln!("file link creation unsupported; skipping export-link test");
        return;
    }

    assert!(zip_dir(&dir, "demo")
        .unwrap_err()
        .0
        .contains("链接或重解析点"));
}

#[test]
fn unpack_rejects_malformed_payload() {
    for bad in [
        None,
        Some(serde_json::json!("demo.zip")),
        Some(serde_json::json!({ "filename": "demo.zip" })),
        Some(serde_json::json!({ "b64": "aGk=" })),
    ] {
        let e = VdfsUnpack::from_payload(bad.as_ref()).unwrap_err();
        assert!(e.0.contains("filename + b64"), "错误要说清缺什么：{}", e.0);
    }
}
