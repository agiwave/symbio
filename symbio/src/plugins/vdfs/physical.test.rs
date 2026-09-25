//! `symbio/src/plugins/vdfs/physical.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::VdfsContext;

fn ctx_in(dir: &Path) -> VdfsContext {
    VdfsContext::empty().with_param(
        VDFS_PARAM_WORKDIR,
        serde_json::Value::String(dir.to_string_lossy().into_owned()),
    )
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "symbio-physical-{tag}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn writes_then_reads_back_relative_address() {
    let base = temp("rw");
    let fs = PhysicalFs::new();
    let ctx = ctx_in(&base);

    fs.dispatch(
        &ctx,
        "a.txt",
        VdfsRequest::Write {
            content: VdfsContent::text("hello"),
        },
    )
    .await
    .unwrap();
    let VdfsResponse::Read(c) = fs.dispatch(&ctx, "a.txt", VdfsRequest::Read).await.unwrap() else {
        panic!("应为 Read 响应");
    };
    assert_eq!(c.text.as_deref(), Some("hello"));
    // 相对与带前导斜杠的写法落在同一位置
    let VdfsResponse::Read(c) = fs
        .dispatch(&ctx, "/a.txt", VdfsRequest::Read)
        .await
        .unwrap()
    else {
        panic!("应为 Read 响应");
    };
    assert_eq!(c.text.as_deref(), Some("hello"));
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn write_creates_missing_parents() {
    let base = temp("parents");
    let fs = PhysicalFs::new();
    let ctx = ctx_in(&base);

    let VdfsResponse::Write(r) = fs
        .dispatch(
            &ctx,
            "x/y/z.txt",
            VdfsRequest::Write {
                content: VdfsContent::text("deep"),
            },
        )
        .await
        .unwrap()
    else {
        panic!("应为 Write 响应");
    };
    assert!(r.created);
    assert!(base.join("x/y/z.txt").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn list_puts_dirs_first_then_sorts_by_name() {
    let base = temp("list");
    let fs = PhysicalFs::new();
    let ctx = ctx_in(&base);
    std::fs::create_dir_all(base.join("zeta")).unwrap();
    std::fs::write(base.join("beta.txt"), "b").unwrap();
    std::fs::write(base.join("alpha.txt"), "a").unwrap();

    let VdfsResponse::List(items) = fs
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
    else {
        panic!("应为 List 响应");
    };
    let names: Vec<&str> = items.iter().map(|it| it.node.name.as_str()).collect();
    assert_eq!(names, vec!["zeta", "alpha.txt", "beta.txt"]);
    assert!(items[0].node.is_dir());
    assert_eq!(items[1].node.size, Some(1));
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn directory_is_not_readable_as_content() {
    let base = temp("dirread");
    let fs = PhysicalFs::new();
    let ctx = ctx_in(&base);
    std::fs::create_dir_all(base.join("sub")).unwrap();

    let err = fs
        .dispatch(&ctx, "sub", VdfsRequest::Read)
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::Forbidden(_)));
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn delete_dir_requires_recursive() {
    let base = temp("del");
    let fs = PhysicalFs::new();
    let ctx = ctx_in(&base);
    std::fs::create_dir_all(base.join("sub")).unwrap();
    std::fs::write(base.join("sub/in.txt"), "x").unwrap();

    assert!(fs
        .dispatch(&ctx, "sub", VdfsRequest::Delete { recursive: false })
        .await
        .is_err());
    fs.dispatch(&ctx, "sub", VdfsRequest::Delete { recursive: true })
        .await
        .unwrap();
    assert!(!base.join("sub").exists());
    let _ = std::fs::remove_dir_all(&base);
}

// 曾经这里有一例 `move_renames_within_workdir`（物理盘用 `rename` 改名）。移动
// 整条下线后 `PhysicalFs` 不再实现它——它正是「只有物理盘能真做」的那一个实现，
// 也就是当初把 `Move` 放进 trait 的唯一理由。删掉它，正是这次收敛的目的。

#[tokio::test]
async fn missing_workdir_param_is_internal_error() {
    let fs = PhysicalFs::new();
    let err = fs
        .dispatch(
            &VdfsContext::empty(),
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, VdfsError::Internal(_)),
        "接线错误不是用户错误"
    );
}

/// 黑名单前缀命中即拒（读与写同规则）
#[test]
fn forbidden_prefix_blocks_both_directions() {
    let p = FsPolicy::default();
    let ws = PathBuf::from("/tmp/ws");
    assert!(!p.readable(Path::new("/etc/passwd"), &ws));
    assert!(!p.writable(Path::new("/etc/passwd"), &ws));
    assert!(p.readable(Path::new("/tmp/ok.txt"), &ws));
}

/// `workspace_only` 开启后，工作区外的绝对路径被拒、白名单根仍放行
#[test]
fn workspace_only_confines_absolute_paths() {
    let ws = temp("ws");
    let outside = temp("outside");
    let p = FsPolicy {
        workspace_only: true,
        ..Default::default()
    };
    assert!(p.readable(&ws.join("a.txt"), &ws));
    assert!(!p.readable(&outside.join("a.txt"), &ws));

    let allowed = FsPolicy {
        workspace_only: true,
        allowed_roots: vec![outside.clone()],
        ..Default::default()
    };
    assert!(allowed.readable(&outside.join("a.txt"), &ws));
    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&outside);
}

// `..` 穿越与黑名单前缀的判定规则测试在 `symbio_core::vdfs`
// （规则只有一份实现，测试随之只有一份）。这里只验**策略集成**：
// 规则接上 `FsPolicy` 之后，哪些路径被真的拦住。

/// 黑名单命中即拒：`/etcfoo` 放行、`/etc` 拦截
#[test]
fn blacklist_rejects_only_real_hits() {
    let p = FsPolicy::default();
    let ws = temp("ws");
    assert!(!p.readable(Path::new("/etc/passwd"), &ws));
    assert!(p.readable(Path::new("/etcfoo/a.txt"), &ws));
    let _ = std::fs::remove_dir_all(&ws);
}

/// 策略集成守卫补全：`..` 逃逸、完整黑名单清单、allowed_roots 前缀边界。
///
/// 规则本身在 `symbio_core::vdfs` 只有一份（`has_parent_segment` /
/// `path_within`），这里验的是 `FsPolicy` 把规则接对——任一处漏接，下面任一
/// 断言都会红，从而暴露物理层安全边界被悄悄放开。
#[test]
fn policy_guards_traversal_blacklist_and_root_boundary() {
    let ws = temp("guard");
    let sibling = temp("guard-sibling"); // 与 ws 同父的兄弟目录，用于前缀边界

    // 1) `..` 逃逸：任何段为 `..` 即拒（读与写同规则）
    let p = FsPolicy::default();
    assert!(!p.readable(Path::new("/tmp/ws/../etc/passwd"), &ws));
    assert!(!p.writable(Path::new("a/../b.txt"), &ws));

    // 2) 完整黑名单清单：默认六条前缀全部命中即拒（含 `~` 展开后的家目录路径）
    for forbidden in [
        "/etc/passwd",
        "/root/.bashrc",
        "/usr/bin/env",
        shellexpand::tilde("~/.ssh/id_rsa").as_ref(),
        shellexpand::tilde("~/.aws/credentials").as_ref(),
    ] {
        assert!(
            !p.readable(Path::new(forbidden), &ws),
            "黑名单应命中：{forbidden}"
        );
    }

    // 3) allowed_roots 按路径段比较：兄弟目录不被误放行
    let allowed = FsPolicy {
        workspace_only: true,
        allowed_roots: vec![ws.clone()],
        ..Default::default()
    };
    assert!(allowed.readable(&ws.join("a.txt"), &ws));
    assert!(
        !allowed.readable(&sibling.join("a.txt"), &ws),
        "allowed_roots 是路径段前缀，兄弟目录不得越权"
    );

    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&sibling);
}
