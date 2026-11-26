//! `crates/symbio-plugin-vdfs/src/physical.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use symbio_plugin_api::VdfsContext;

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

fn workspace_fs() -> PhysicalFs {
    PhysicalFs {
        policy: FsPolicy {
            workspace_only: true,
            allowed_roots: Vec::new(),
            forbidden_paths: Vec::new(),
        },
    }
}

#[cfg(unix)]
fn directory_symlink(link: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn try_directory_junction(link: &Path, target: &Path) -> bool {
    use std::os::windows::process::CommandExt as _;
    // cmd 的 /C 只有一个命令字符串参数；交给 Command 逐个 quoting 会把内层引号转义掉。
    let command = format!("mklink /J \"{}\" \"{}\"", link.display(), target.display());
    std::process::Command::new("cmd")
        .args(["/D", "/C"])
        .raw_arg(command)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    // `docs` 与 `/docs` 是两个位置：首部 `/` 是绝对锚点，落点不在工作目录里
    let anchored = fs
        .dispatch(&ctx, "/symbio-anchor-probe-a7c1.txt", VdfsRequest::Read)
        .await;
    assert!(
        matches!(anchored, Err(VdfsError::NotFound(_))),
        "锚点地址不该命中工作区里的 a.txt：{anchored:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// 地址 → 真实路径的那一条规则：相对段从工作目录起算，`/` 打头交给操作系统解析
#[test]
fn leading_slash_is_an_absolute_anchor() {
    let base = Path::new(if cfg!(windows) {
        r"C:\work\proj"
    } else {
        "/work/proj"
    });
    assert_eq!(join_target(base, "docs"), base.join("docs"));
    let anchored = join_target(base, "/docs");
    assert!(anchored.has_root());
    assert!(
        !anchored.starts_with(base),
        "锚点地址不该落进工作目录：{anchored:?}"
    );
    if cfg!(windows) {
        assert_eq!(
            join_target(base, r"D:\tmp\a.txt"),
            PathBuf::from(r"D:\tmp\a.txt")
        );
    }
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

/// 工作区限制必须覆盖“目标尚不存在”的嵌套父目录。
#[tokio::test]
async fn workspace_only_allows_safe_nonexistent_nested_parents() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    std::fs::create_dir_all(&base).unwrap();
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    fs.dispatch(
        &ctx,
        "new/deep/file.txt",
        VdfsRequest::Write {
            content: VdfsContent::text("safe"),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(base.join("new/deep/file.txt")).unwrap(),
        "safe"
    );
}

/// Unix：父目录链接不能把不存在的后续组件写到工作区外；删 / 建目录同样拒绝。
#[cfg(unix)]
#[tokio::test]
async fn unix_parent_symlink_escape_is_rejected_for_all_mutations() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("sentinel"), "keep").unwrap();
    directory_symlink(&base.join("escape"), &outside).unwrap();
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    let write = fs
        .dispatch(
            &ctx,
            "escape/new/deep.txt",
            VdfsRequest::Write {
                content: VdfsContent::text("owned"),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(write, VdfsError::Forbidden(_)), "{write:?}");
    assert!(!outside.join("new/deep.txt").exists());

    assert!(fs
        .dispatch(&ctx, "escape", VdfsRequest::Delete { recursive: true })
        .await
        .is_err());
    assert!(fs
        .dispatch(&ctx, "escape/created", VdfsRequest::Mkdir)
        .await
        .is_err());
    assert!(!outside.join("created").exists());
    assert_eq!(
        std::fs::read_to_string(outside.join("sentinel")).unwrap(),
        "keep"
    );
}

/// Unix：目标本身是符号链接时，write / delete / mkdir 都不能操作链接指向的文件。
#[cfg(unix)]
#[tokio::test]
async fn unix_target_symlink_is_rejected_for_all_mutations() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    let outside = tmp.path().join("outside.txt");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(&outside, "keep").unwrap();
    std::os::unix::fs::symlink(&outside, base.join("link.txt")).unwrap();
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    assert!(fs
        .dispatch(
            &ctx,
            "link.txt",
            VdfsRequest::Write {
                content: VdfsContent::text("changed"),
            },
        )
        .await
        .is_err());
    assert!(fs
        .dispatch(&ctx, "link.txt", VdfsRequest::Delete { recursive: false })
        .await
        .is_err());
    assert!(fs
        .dispatch(&ctx, "link.txt", VdfsRequest::Mkdir)
        .await
        .is_err());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
}

/// Windows：junction 父目录必须按重解析点拒绝，不能靠 canonicalize 跟随。
#[cfg(windows)]
#[tokio::test]
async fn windows_junction_parent_escape_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    if !try_directory_junction(&base.join("escape"), &outside) {
        eprintln!("junction creation unsupported; skipping junction escape test");
        return;
    }
    let fs = workspace_fs();
    let ctx = ctx_in(&base);
    assert!(fs
        .dispatch(
            &ctx,
            "escape/new/file.txt",
            VdfsRequest::Write {
                content: VdfsContent::text("owned"),
            },
        )
        .await
        .is_err());
    assert!(!outside.join("new/file.txt").exists());
    assert!(fs
        .dispatch(&ctx, "escape", VdfsRequest::Delete { recursive: true })
        .await
        .is_err());
    assert!(fs
        .dispatch(&ctx, "escape/new", VdfsRequest::Mkdir)
        .await
        .is_err());
}

/// Windows：有权限创建符号链接时，目标链接同样不能绕过三种变更操作。
#[cfg(windows)]
#[tokio::test]
async fn windows_target_symlink_is_rejected_when_supported() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    let outside = tmp.path().join("outside.txt");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(&outside, "keep").unwrap();
    if let Err(e) = std::os::windows::fs::symlink_file(&outside, base.join("link.txt")) {
        eprintln!("directory symlink creation unsupported: {e}");
        return;
    }
    let fs = workspace_fs();
    let ctx = ctx_in(&base);
    assert!(fs
        .dispatch(
            &ctx,
            "link.txt",
            VdfsRequest::Write {
                content: VdfsContent::text("changed"),
            },
        )
        .await
        .is_err());
    assert!(fs
        .dispatch(&ctx, "link.txt", VdfsRequest::Delete { recursive: false })
        .await
        .is_err());
    assert!(fs
        .dispatch(&ctx, "link.txt", VdfsRequest::Mkdir)
        .await
        .is_err());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
}

#[cfg(not(windows))]
fn try_directory_link(target: &Path, link: &Path) -> bool {
    directory_symlink(target, link).is_ok()
}

#[cfg(windows)]
fn try_directory_link(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok() || try_directory_junction(link, target)
}

#[tokio::test]
async fn linked_base_or_base_ancestor_is_rejected_before_every_operation() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    let parent = tmp.path().join("parent");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "keep").unwrap();
    if !try_directory_link(&outside, &parent) {
        eprintln!("directory link creation unsupported; skipping base ancestor test");
        return;
    }
    let base = parent.join("workspace");
    std::fs::create_dir_all(&base).unwrap();
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    assert!(matches!(
        fs.dispatch(&ctx, "keep.txt", VdfsRequest::Read).await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        fs.dispatch(&ctx, "", VdfsRequest::List {}).await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        fs.dispatch(
            &ctx,
            "new.txt",
            VdfsRequest::Write {
                content: VdfsContent::text("owned")
            }
        )
        .await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        fs.dispatch(&ctx, "keep.txt", VdfsRequest::Delete { recursive: false })
            .await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        fs.dispatch(&ctx, "new-dir", VdfsRequest::Mkdir).await,
        Err(VdfsError::Forbidden(_))
    ));
    assert_eq!(
        std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "keep"
    );
    assert!(!outside.join("new.txt").exists());
    assert!(!outside.join("new-dir").exists());
}

#[tokio::test]
async fn reads_reject_links_even_when_the_target_is_inside_the_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    std::fs::create_dir_all(base.join("denied")).unwrap();
    std::fs::write(base.join("denied/secret.txt"), "secret").unwrap();
    if !try_directory_link(&base.join("denied"), &base.join("link")) {
        eprintln!("directory link creation unsupported; skipping read-link test");
        return;
    }
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    assert!(matches!(
        fs.dispatch(&ctx, "link/secret.txt", VdfsRequest::Read)
            .await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        fs.dispatch(&ctx, "link", VdfsRequest::List {}).await,
        Err(VdfsError::Forbidden(_))
    ));
}

#[tokio::test]
async fn write_replaces_the_file_instead_of_truncating_it_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    std::fs::create_dir_all(&base).unwrap();
    let target = base.join("a.txt");
    let old_link = base.join("old-view.txt");
    std::fs::write(&target, "old complete").unwrap();
    std::fs::hard_link(&target, &old_link).unwrap();
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    fs.dispatch(
        &ctx,
        "a.txt",
        VdfsRequest::Write {
            content: VdfsContent::text("new complete"),
        },
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new complete");
    assert_eq!(std::fs::read_to_string(&old_link).unwrap(), "old complete");
    let names: Vec<_> = std::fs::read_dir(&base)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "原子写不得留下临时文件");
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_replacement_preserves_existing_file_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = temp("mode");
    let target = base.join("a.txt");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    fs.dispatch(
        &ctx,
        "a.txt",
        VdfsRequest::Write {
            content: VdfsContent::text("new"),
        },
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(windows)]
#[tokio::test]
async fn atomic_replacement_preserves_existing_file_acl() {
    let base = temp("windows-acl");
    let target = base.join("a.txt");
    std::fs::write(&target, "old").unwrap();
    let status = std::process::Command::new("icacls")
        .arg(&target)
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
    let before = acl(&target);
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    fs.dispatch(
        &ctx,
        "a.txt",
        VdfsRequest::Write {
            content: VdfsContent::text("new"),
        },
    )
    .await
    .unwrap();
    assert_eq!(acl(&target), before);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn recursive_delete_rejects_nested_links_before_deleting_any_child() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(base.join("tree/nested")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(base.join("tree/first.txt"), "keep").unwrap();
    std::fs::write(outside.join("sentinel"), "keep").unwrap();
    if !try_directory_link(&outside, &base.join("tree/nested/escape")) {
        eprintln!("directory link creation unsupported; skipping recursive delete test");
        return;
    }
    let fs = workspace_fs();
    let ctx = ctx_in(&base);

    assert!(matches!(
        fs.dispatch(&ctx, "tree", VdfsRequest::Delete { recursive: true })
            .await,
        Err(VdfsError::Forbidden(_))
    ));
    assert_eq!(
        std::fs::read_to_string(base.join("tree/first.txt")).unwrap(),
        "keep"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("sentinel")).unwrap(),
        "keep"
    );
}

#[tokio::test]
async fn concurrent_reads_and_writes_never_observe_partial_content() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    std::fs::create_dir_all(&base).unwrap();
    let fs = workspace_fs();
    let ctx = ctx_in(&base);
    let payloads: std::sync::Arc<Vec<String>> = std::sync::Arc::new(
        (0..8)
            .map(|i| format!("{i:02}:{}:complete", "z".repeat(96 * 1024)))
            .collect(),
    );
    let mut tasks = Vec::new();
    for payload in payloads.iter() {
        let fs = PhysicalFs {
            policy: fs.policy.clone(),
        };
        let ctx = ctx.clone();
        let payload = payload.clone();
        tasks.push(tokio::spawn(async move {
            fs.dispatch(
                &ctx,
                "shared.txt",
                VdfsRequest::Write {
                    content: VdfsContent::text(payload),
                },
            )
            .await
            .unwrap();
        }));
    }
    for _ in 0..4 {
        let fs = PhysicalFs {
            policy: fs.policy.clone(),
        };
        let ctx = ctx.clone();
        let expected = payloads.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..32 {
                let VdfsResponse::Read(content) = fs
                    .dispatch(&ctx, "shared.txt", VdfsRequest::Read)
                    .await
                    .unwrap()
                else {
                    panic!("应为 Read 响应");
                };
                assert!(expected.contains(&content.text.unwrap()));
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
}

#[test]
fn temp_file_guard_removes_only_its_owned_path() {
    let tmp = tempfile::tempdir().unwrap();
    let owned = tmp.path().join("owned.tmp");
    let other = tmp.path().join("other.tmp");
    std::fs::write(&owned, "partial").unwrap();
    std::fs::write(&other, "keep").unwrap();
    let mut guard = TempFileGuard::new(owned.clone());
    guard.arm();
    drop(guard);
    assert!(!owned.exists());
    assert_eq!(std::fs::read_to_string(other).unwrap(), "keep");
}

/// 兄弟目录只共享字符串前缀，不共享路径段前缀。
#[tokio::test]
async fn workspace_only_rejects_sibling_prefix_on_actual_write() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("workspace");
    let sibling = tmp.path().join("workspace-other");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let fs = workspace_fs();
    let ctx = ctx_in(&base);
    let addr = sibling.join("escape.txt").to_string_lossy().into_owned();
    assert!(fs
        .dispatch(
            &ctx,
            &addr,
            VdfsRequest::Write {
                content: VdfsContent::text("owned"),
            },
        )
        .await
        .is_err());
    assert!(!sibling.join("escape.txt").exists());
}

#[cfg(windows)]
#[test]
fn windows_policy_comparison_normalizes_case_and_verbatim_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("Workspace");
    std::fs::create_dir_all(&base).unwrap();
    let base_text = base.to_string_lossy();
    let verbatim = PathBuf::from(format!(r"\\?\{base_text}"));
    let upper = PathBuf::from(base_text.to_ascii_uppercase()).join("New/File.txt");
    let p = workspace_fs().policy;
    assert!(p.writable(&verbatim.join("new/file.txt"), &base));
    assert!(p.writable(&upper, &PathBuf::from(base_text.to_ascii_lowercase())));

    let forbidden = FsPolicy {
        forbidden_paths: vec![verbatim.to_string_lossy().into_owned()],
        ..Default::default()
    };
    assert!(!forbidden.writable(&upper, &base));
    assert_eq!(
        normalize_for_comparison(Path::new(r"\\?\UNC\Server\Share\Dir")),
        PathBuf::from(r"\\server\share\dir")
    );
}

#[tokio::test]
async fn list_puts_dirs_first_then_sorts_by_name() {
    let base = temp("list");
    let fs = PhysicalFs::new();
    let ctx = ctx_in(&base);
    std::fs::create_dir_all(base.join("zeta")).unwrap();
    std::fs::write(base.join("beta.txt"), "b").unwrap();
    std::fs::write(base.join("alpha.txt"), "a").unwrap();

    let VdfsResponse::List(items) = fs.dispatch(&ctx, "", VdfsRequest::List {}).await.unwrap()
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

// `move_renames_within_workdir`（物理盘侧 `rename` 改名）已随移动整条下线删除。
// 整条下线后 `PhysicalFs` 不再实现它——它正是「只有物理盘能真做」的那一个实现，
// 也就是当初把 `Move` 放进 trait 的唯一理由。删掉它，正是这次收敛的目的。

#[tokio::test]
async fn missing_workdir_param_is_internal_error() {
    let fs = PhysicalFs::new();
    let err = fs
        .dispatch(&VdfsContext::empty(), "", VdfsRequest::List {})
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

// `..` 穿越与黑名单前缀的判定规则测试在 `symbio_plugin_api::vdfs`
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
/// 规则本身在 `symbio_plugin_api::vdfs` 只有一份（`vdfs_has_parent_segment` /
/// `vdfs_path_within`），这里验的是 `FsPolicy` 把规则接对——任一处漏接，下面任一
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
