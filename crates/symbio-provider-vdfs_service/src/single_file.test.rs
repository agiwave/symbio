//! `crates/symbio-provider-vdfs_service/src/single_file.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use std::path::Path;
use symbio_kernel::vdfs::VdfsContent;

fn store_in(base: &Path) -> SingleFileVdfs {
    SingleFileVdfs::at(base, "model", "provider.json").with_label("模型")
}

#[tokio::test]
async fn writes_then_reads_the_main_file_only() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());

    assert!(s.entries().await.unwrap().is_empty(), "空类别是合法状态");
    assert!(s.write_text("p1", "{\"id\":\"p1\"}").await.unwrap());
    assert_eq!(s.read_text("p1").await.unwrap(), "{\"id\":\"p1\"}");

    // 条目内部不进地址空间：主文件之外的文件列不出来
    std::fs::create_dir_all(s.entry_dir("p1").join("secret")).unwrap();
    std::fs::write(s.entry_dir("p1").join("secret/note.txt"), b"x").unwrap();
    let err = s
        .dispatch(&VdfsContext::empty(), "p1", VdfsRequest::List {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, VdfsError::NotFound(_)),
        "单文件型不得暴露条目内部：{err:?}"
    );
    assert_eq!(s.entries().await.unwrap().len(), 1);
}

/// 磁盘布局不变：`<base>/<category>/<id>/<manifest>`
#[tokio::test]
async fn keeps_the_existing_disk_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(&tmp.path().join("model"));
    s.write_text("p1", "{}").await.unwrap();
    assert!(tmp.path().join("model/p1/provider.json").exists());
}

#[tokio::test]
async fn default_provider_ops_work_without_any_differential() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    let ctx = VdfsContext::empty();

    let _ = s
        .dispatch(
            &ctx,
            "p1.model",
            VdfsRequest::Write {
                content: VdfsContent::text("{\"id\":\"p1\"}"),
            },
        )
        .await
        .unwrap();
    // 呈现扩展名不是地址的一部分
    assert_eq!(
        s.dispatch(&ctx, "p1", VdfsRequest::Read)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .as_text(),
        Some("{\"id\":\"p1\"}")
    );
    let node = s
        .dispatch(&ctx, "p1.model", VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
    assert_eq!(node.name, "p1");
    assert_eq!(node.ext.as_deref(), Some("json"));
    assert_eq!(node.access, VdfsAccess::READ_WRITE);

    let listed = s
        .dispatch(&ctx, "", VdfsRequest::List {})
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|it| it.node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["p1"]
    );

    assert!(matches!(
        s.dispatch(&ctx, "nope", VdfsRequest::Delete { recursive: false })
            .await
            .unwrap_err(),
        VdfsError::NotFound(_)
    ));
    let _ = s
        .dispatch(&ctx, "p1", VdfsRequest::Delete { recursive: false })
        .await
        .unwrap();
    assert!(s.entries().await.unwrap().is_empty());
}

/// 删不存在的条目：落盘侧幂等告警（内存同步仍要跑到）
#[tokio::test]
async fn remove_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    s.remove("ghost").await.unwrap();
}

/// 导出 → 导入原样往返（导出的包能直接导回）
#[tokio::test]
async fn pack_roundtrips_through_the_binary_channel() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    s.write_text("p1", "{\"id\":\"p1\"}").await.unwrap();
    let pack = s.export_pack("p1").await.unwrap();
    let bytes = super::super::pack::decode_b64(&pack.b64).unwrap();

    s.write_text("p1", "stale").await.unwrap();
    assert!(
        !s.import_pack("p1", &bytes).await.unwrap(),
        "同名导入是覆盖，不是新建"
    );
    assert_eq!(s.read_text("p1").await.unwrap(), "{\"id\":\"p1\"}");
}

/// 克隆与临时重建的 store 共享 base 锁；并发保存后只能是完整内容之一。
#[tokio::test]
async fn cloned_and_reconstructed_stores_serialize_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("model");
    let mut tasks = Vec::new();
    for i in 0..16 {
        // 每个任务都重新构造，而不是依赖某个实例字段里的锁。
        let store = store_in(&base);
        let payload = format!("{i:02}:{}:complete", "y".repeat(48 * 1024));
        tasks.push(tokio::spawn(async move {
            store.write_text("p1", &payload).await.unwrap();
            payload
        }));
    }
    let mut expected = Vec::new();
    for task in tasks {
        expected.push(task.await.unwrap());
    }

    let store = store_in(&base);
    let actual = store.read_text("p1").await.unwrap();
    assert!(expected.contains(&actual));
    let names: Vec<_> = std::fs::read_dir(store.entry_dir("p1"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("provider.json")]);
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write as _;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }
    cursor.into_inner()
}

#[tokio::test]
async fn export_rejects_linked_entry_and_linked_children() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    let s = store_in(&tmp.path().join("model"));
    s.write_text("p1", "{}").await.unwrap();

    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, s.entry_dir("p1").join("escape")).is_ok();
    #[cfg(windows)]
    let linked =
        std::os::windows::fs::symlink_dir(&outside, s.entry_dir("p1").join("escape")).is_ok() || {
            use std::os::windows::process::CommandExt as _;
            let command = format!(
                "mklink /J \"{}\" \"{}\"",
                s.entry_dir("p1").join("escape").display(),
                outside.display()
            );
            std::process::Command::new("cmd")
                .args(["/D", "/C"])
                .raw_arg(command)
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        };
    if !linked {
        eprintln!("directory link creation unsupported; skipping export link test");
        return;
    }

    assert!(matches!(
        s.export_pack("p1").await,
        Err(VdfsError::Forbidden(_))
    ));
}

#[tokio::test]
async fn failed_import_keeps_old_entry_and_removes_private_staging() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(&tmp.path().join("model"));
    s.write_text("p1", "old complete").await.unwrap();
    std::fs::write(s.entry_dir("p1").join("stale.txt"), "stale").unwrap();
    let bytes = zip_bytes(&[("a", b"file"), ("a/b.txt", b"conflict")]);

    assert!(s.import_pack("p1", &bytes).await.is_err());
    assert_eq!(s.read_text("p1").await.unwrap(), "old complete");
    assert!(s.entry_dir("p1").join("stale.txt").exists());
    let leftovers: Vec<_> = std::fs::read_dir(&s.base)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("p1")]);
}

#[tokio::test]
async fn successful_import_replaces_whole_entry_and_leaves_no_staging() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(&tmp.path().join("model"));
    s.write_text("p1", "old").await.unwrap();
    std::fs::write(s.entry_dir("p1").join("stale.txt"), "stale").unwrap();
    let bytes = zip_bytes(&[
        ("p1/provider.json", b"new complete"),
        ("p1/sub/a.txt", b"a"),
    ]);

    assert!(!s.import_pack("p1", &bytes).await.unwrap());
    assert_eq!(s.read_text("p1").await.unwrap(), "new complete");
    assert_eq!(
        std::fs::read_to_string(s.entry_dir("p1").join("sub/a.txt")).unwrap(),
        "a"
    );
    assert!(!s.entry_dir("p1").join("stale.txt").exists());
    let leftovers: Vec<_> = std::fs::read_dir(&s.base)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("p1")]);
}

#[test]
fn import_guard_restores_old_entry_and_cleans_only_its_staging() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".private-staging");
    let target = tmp.path().join("p1");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("provider.json"), "old").unwrap();
    std::fs::create_dir_all(root.join("new")).unwrap();
    std::fs::write(root.join("new/provider.json"), "partial").unwrap();
    let mut guard = ImportGuard::new(root.clone(), target.clone());
    std::fs::rename(&target, &guard.old).unwrap();
    guard.old_moved = true;
    drop(guard);

    assert_eq!(
        std::fs::read_to_string(target.join("provider.json")).unwrap(),
        "old"
    );
    assert!(!root.exists());
}

#[test]
fn import_guard_preserves_the_only_old_copy_when_rollback_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".private-staging");
    let target = tmp.path().join("p1");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("provider.json"), "old").unwrap();
    let mut guard = ImportGuard::new(root.clone(), target.clone());
    std::fs::rename(&target, &guard.old).unwrap();
    guard.old_moved = true;
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("unexpected"), "conflict").unwrap();

    drop(guard);

    assert_eq!(
        std::fs::read_to_string(root.join("old/provider.json")).unwrap(),
        "old"
    );
}

/// 未声明的操作保持 `NotImplemented`（机制据此隐藏入口）
#[tokio::test]
async fn unsupported_ops_stay_not_implemented() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    let ctx = VdfsContext::empty();
    assert!(s
        .dispatch(&ctx, "d", VdfsRequest::Mkdir)
        .await
        .unwrap_err()
        .is_not_implemented());
    assert!(s
        .dispatch(
            &ctx,
            "p1",
            VdfsRequest::Action {
                action: "test".to_string(),
                payload: None
            }
        )
        .await
        .unwrap_err()
        .is_not_implemented());
}
