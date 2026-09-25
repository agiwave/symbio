//! `symbio/src/providers/vdfs_service/single_file.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::VdfsContent;
use std::path::Path;

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
        .dispatch(
            &VdfsContext::empty(),
            "p1",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
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
        .dispatch(
            &ctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
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
