//! `symbio/src/providers/vdfs_service/dir.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use std::path::Path;

fn store_in(base: &Path) -> DirVdfs {
    DirVdfs::at(base, "skill", "SKILL.md").with_label("技能")
}

#[tokio::test]
async fn entry_is_a_drillable_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(&tmp.path().join("skill"));
    let ctx = VdfsContext::empty();

    let _ = s
        .dispatch(
            &ctx,
            "demo.skill",
            VdfsRequest::Write {
                content: VdfsContent::text("", "# demo"),
            },
        )
        .await
        .unwrap();
    assert!(tmp.path().join("skill/demo/SKILL.md").exists());

    // 条目内部进地址空间：写一个附属文件，再从根下钻读回
    let _ = s
        .dispatch(
            &ctx,
            "demo/scripts/run.sh",
            VdfsRequest::Write {
                content: VdfsContent::text("", "echo hi"),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        s.dispatch(&ctx, "demo/scripts/run.sh", VdfsRequest::Read)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .as_text(),
        Some("echo hi")
    );
    let items = s
        .dispatch(
            &ctx,
            "demo",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["scripts", "SKILL.md"], "目录在前、按名升序");

    // 根清单把条目呈现为**目录**（可下钻），不是叶子文件
    let root = s
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
    assert_eq!(root.len(), 1);
    assert!(root[0].is_dir());
    assert_eq!(root[0].name, "demo");
    assert!(root[0].access.traverse);
}

#[tokio::test]
async fn non_ascii_text_degrades_to_binary_channel() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    std::fs::create_dir_all(s.entry_dir("demo")).unwrap();
    std::fs::write(s.inner_path("demo", "img.bin"), [0u8, 159, 1, 2]).unwrap();
    let c = s
        .dispatch(&VdfsContext::empty(), "demo/img.bin", VdfsRequest::Read)
        .await
        .unwrap()
        .into_read()
        .unwrap();
    assert!(c.binary, "非 UTF-8 内容必须走二进制通道，而不是报错");
}

/// 导出 → 导入往返，且导入即整目录覆盖（附属文件不残留）
#[tokio::test]
async fn pack_roundtrip_replaces_whole_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    let ctx = VdfsContext::empty();
    let _ = s
        .dispatch(
            &ctx,
            "demo",
            VdfsRequest::Write {
                content: VdfsContent::text("", "# demo"),
            },
        )
        .await
        .unwrap();
    let _ = s
        .dispatch(
            &ctx,
            "demo/extra.txt",
            VdfsRequest::Write {
                content: VdfsContent::text("", "x"),
            },
        )
        .await
        .unwrap();

    let r = s
        .dispatch(
            &ctx,
            "demo",
            VdfsRequest::Action {
                action: VDFS_ACTION_EXPORT.to_string(),
                payload: None,
            },
        )
        .await
        .unwrap()
        .into_action()
        .unwrap();
    assert!(r.ok);
    let b64 = r.data.unwrap()["b64"].as_str().unwrap().to_string();
    let bytes = super::super::pack::decode_b64(&b64).unwrap();

    // 换一个只含主文件的包导回：附属文件必须消失
    let _ = s
        .dispatch(
            &ctx,
            "other",
            VdfsRequest::Write {
                content: VdfsContent::text("", "# only"),
            },
        )
        .await
        .unwrap();
    let only = s.export_pack("other").await.unwrap();
    s.import_pack("demo", &super::super::pack::decode_b64(&only.b64).unwrap())
        .await
        .unwrap();
    assert!(
        !s.inner_path("demo", "extra.txt").exists(),
        "导入即替换：旧包里没覆盖到的文件不得残留"
    );
    assert_eq!(s.read_text("demo").await.unwrap(), "# only");
    let _ = bytes.len();
}

/// 带子结构的条目非 `recursive` 不得删除
#[tokio::test]
async fn delete_with_children_needs_recursive() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    let ctx = VdfsContext::empty();
    let _ = s
        .dispatch(
            &ctx,
            "demo",
            VdfsRequest::Write {
                content: VdfsContent::text("", "# demo"),
            },
        )
        .await
        .unwrap();
    let _ = s
        .dispatch(
            &ctx,
            "demo/a.txt",
            VdfsRequest::Write {
                content: VdfsContent::text("", "x"),
            },
        )
        .await
        .unwrap();
    assert!(s
        .dispatch(&ctx, "demo", VdfsRequest::Delete { recursive: false })
        .await
        .is_err());
    let _ = s
        .dispatch(&ctx, "demo", VdfsRequest::Delete { recursive: true })
        .await
        .unwrap();
    assert!(!s.exists("demo"));
}

#[tokio::test]
async fn mkdir_conflicts_on_existing_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    let ctx = VdfsContext::empty();
    let _ = s.dispatch(&ctx, "demo", VdfsRequest::Mkdir).await.unwrap();
    assert!(matches!(
        s.dispatch(&ctx, "demo", VdfsRequest::Mkdir)
            .await
            .unwrap_err(),
        VdfsError::Conflict(_)
    ));
    // 空条目（无主文件）读不到内容，但列得出来
    assert!(s.read_text("demo").await.is_err());
    assert_eq!(s.entries().await.unwrap().len(), 1);
}

/// 呈现扩展名不是地址的一部分：`demo` / `demo.skill` 同解
#[tokio::test]
async fn presentation_ext_is_not_part_of_the_address() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    let ctx = VdfsContext::empty();
    let _ = s
        .dispatch(
            &ctx,
            "demo.skill",
            VdfsRequest::Write {
                content: VdfsContent::text("", "# a"),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        s.dispatch(&ctx, "demo", VdfsRequest::Read)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .as_text(),
        Some("# a")
    );
    assert_eq!(
        s.dispatch(&ctx, "demo.skill", VdfsRequest::Stat)
            .await
            .unwrap()
            .into_stat()
            .unwrap()
            .name,
        "demo"
    );
}

/// 广播频道按 `kind` 全局持有（同一 provider 会被每次 traverse 重新构造），
/// 因此订阅类测试必须用**独占的 kind**，否则与同 binary 内其它测试互相串台。
#[tokio::test]
async fn watch_and_unwatch_are_paired() {
    let tmp = tempfile::tempdir().unwrap();
    let s = DirVdfs::at(tmp.path(), "watch-skill", "SKILL.md");
    let ctx = VdfsContext::empty();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = {
        let seen = seen.clone();
        std::sync::Arc::new(move |c: crate::symbio_core::vdfs_provider::VdfsChange| {
            seen.lock().unwrap().push(c.path);
        })
    };
    let _ = s
        .dispatch(&ctx, "", VdfsRequest::Watch { sink })
        .await
        .unwrap();
    s.write_text("demo", "# x").await.unwrap();
    // 广播是异步投递：给转发任务一点时间
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(seen.lock().unwrap().as_slice(), ["demo"]);
    let _ = s.dispatch(&ctx, "", VdfsRequest::Unwatch).await.unwrap();
    s.write_text("demo", "# y").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(seen.lock().unwrap().len(), 1, "unwatch 后不得再收到事件");
}

/// 反斜杠形式的 `..` 不得逃出条目目录——**不依赖访问层**的纵深防御。
///
/// 访问层 `normalize_addr` 已拦一道，但 provider 可能经别的访问路径被复用
/// （解包、直接构造），因此这条防线必须独立成立。`remove_inner` 用
/// `remove_dir_all`，一旦逃逸即可删除任意目录。
#[tokio::test]
async fn inner_rel_rejects_separator_smuggling() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store_in(tmp.path());
    let ctx = VdfsContext::empty();
    s.write_text("demo", "# x").await.unwrap();

    // 哨兵目录放在条目根之外：证明「若没有守卫就真的会够到」
    let outside = tmp.path().join("vdfs-sentinel");
    std::fs::create_dir_all(&outside).unwrap();

    for bad in [
        r"demo/..\..\escaped.txt",
        r"demo/..\escaped.txt",
        r"demo/sub\..\..\escaped.txt",
        r"demo/a\b.txt",
        r"demo/..\vdfs-sentinel",
    ] {
        for r in [
            s.dispatch(&ctx, bad, VdfsRequest::Read).await.map(|_| ()),
            s.dispatch(
                &ctx,
                bad,
                VdfsRequest::Write {
                    content: VdfsContent::text("", "pwned"),
                },
            )
            .await
            .map(|_| ()),
            s.dispatch(&ctx, bad, VdfsRequest::Delete { recursive: true })
                .await
                .map(|_| ()),
        ] {
            assert!(r.is_err(), "反斜杠路径必须被拒绝：{bad}");
        }
    }

    assert!(outside.exists(), "哨兵目录不得被穿越删除");
    assert!(!tmp.path().join("escaped.txt").exists());
}
