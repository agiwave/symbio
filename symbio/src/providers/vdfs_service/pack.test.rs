//! `symbio/src/providers/vdfs_service/pack.rs` 的单元测试 —— 拆自源码末尾的测试模块。
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

/// zip-slip：包内条目**一律不得**写到目标目录之外
///
/// 两道防线任一生效即可（隐藏段过滤会先丢弃 `..` 段，显式守卫是第二道）；
/// 这里断言的是**可观察性质**而不是走哪条分支——分支属于实现细节。
#[test]
fn parse_pack_never_yields_an_escaping_entry() {
    for hostile in [
        "evil/../../escape.txt",
        "../../escape.txt",
        "a/../../../escape.txt",
    ] {
        let bytes = make_zip(&[(hostile, Some(b"boom"))]);
        let out = parse_pack(&bytes).unwrap_or_default();
        assert!(
            out.iter()
                .all(|(p, _)| !p.split(['/', '\\']).any(|s| s == "..")),
            "条目 `{hostile}` 逃出了目标目录：{out:?}"
        );
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
        ("../../escape2.txt", Some(b"boom")),
    ]);
    extract_pack(&entry_dir, &bytes).await.unwrap();
    assert!(entry_dir.join("SKILL.md").exists());
    assert!(
        !tmp.path().join("escape.txt").exists() && !tmp.path().join("escape2.txt").exists(),
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
}

/// 载荷形状：`filename` + `b64` 就是前端认得的全部（它不认识「导出」）
#[test]
fn pack_payload_is_a_file_shape() {
    let p = VdfsPack::new("demo", b"hi");
    assert_eq!(p.filename, "demo.zip");
    assert_eq!(decode_b64(&p.b64).unwrap(), b"hi".to_vec());
}
