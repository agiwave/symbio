//! `result_guard` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `result_guard.rs` 只保留生产代码，测试全部放本文件。
//!
//!  存档走 `SESSION_VDFS`（根 = 会话目录）：这里用 `ScopeDirVdfs` 绑一个临时根
//! 模拟会话作用域，既验证虚拟地址（提示词里印的那条），也验证它确实落到 provider 根下。

use super::*;
use std::path::{Path, PathBuf};
use symbio_plugin_api::VdfsContext;
use symbio_providers::ScopeDirVdfs;

// 本 crate 的测试二进制不链接 vdfs 插件，取回地址的根名判据需要一条静态声明才测得到。
// 名字取测试专用值——根名字面量只归 vdfs 插件（`fs::VDFS_ADDR_ROOT`），这里不写它。
symbio_plugin_api::inventory::submit! {
    symbio_plugin_api::AddrRootDecl(".test-vdfs-root")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio_guard_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn provider_in(dir: &Path) -> DynVdfsProvider {
    std::sync::Arc::new(ScopeDirVdfs::at(dir.to_path_buf()))
}

async fn list_count(provider: &DynVdfsProvider) -> usize {
    let vctx = VdfsContext::new(());
    provider
        .dispatch(
            &vctx,
            ARCHIVE_REL_DIR,
            symbio_plugin_api::VdfsRequest::List {},
        )
        .await
        .unwrap()
        .into_list()
        .unwrap()
        .len()
}

#[tokio::test]
async fn short_text_passes_through() {
    let g = guard_result("hello world", 8192, None, None).await;
    assert!(!g.truncated);
    assert_eq!(g.text, "hello world");
}

#[tokio::test]
async fn long_text_is_truncated_and_keeps_head_tail() {
    // 构造远超预算的多行文本
    let line = "fn main() {\n    println!(\"line\");\n}\n";
    let big = line.repeat(2000);
    let dir = temp_dir("truncate");
    let provider = provider_in(&dir);
    let g = guard_result(&big, 500, Some(&provider), None).await;
    assert!(g.truncated);
    assert!(g.text.contains("[... 已省略"));
    // 统一取回协议：占位符含存档相对地址与 vdfs_read 取回入口
    assert!(g.text.contains("完整输出已存档至: tool_archives/"));
    assert!(g.text.contains("vdfs_read"));
    // head 与 tail 应分别保留原文片段
    assert!(g.text.contains("fn main()"));
    assert!(g.text.len() < big.len() / 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 单行超长内容：按行截断失效（首行整行放行），应按字符截断行首
#[tokio::test]
async fn single_long_line_is_char_truncated() {
    let huge = format!("{{\"entries\":[\"{}\"]}}", "x".repeat(10_000));
    let dir = temp_dir("singleline");
    let provider = provider_in(&dir);
    let g = guard_result(&huge, 300, Some(&provider), None).await;
    assert!(g.truncated, "单行超预算应触发存档截断");
    assert!(g.text.contains("vdfs_read"), "占位提示应存在");
    assert!(
        g.text.starts_with("{\"entries"),
        "行首应保留: {}",
        &g.text[..40.min(g.text.len())]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 防碰撞：同毫秒同 token 数的不同内容必须落到不同相对地址
#[tokio::test]
async fn same_milli_same_tokens_do_not_collide() {
    let dir = temp_dir("collide");
    let provider = provider_in(&dir);
    // 同长度（≈同 token 数）但内容不同，毫秒内连续写入
    let a = "a".repeat(4096);
    let b = format!("a{}a", "b".repeat(4094));
    let p1 = archive_full_text(&a, 1024, Some(&provider))
        .await
        .expect("写入 a");
    let p2 = archive_full_text(&b, 1024, Some(&provider))
        .await
        .expect("写入 b");
    assert_ne!(p1, p2, "同毫秒同 token 数的不同内容不得共享同一文件名");
    let vctx = VdfsContext::new(());
    for (rel, text) in [(p1, a), (p2, b)] {
        let got = provider
            .dispatch(&vctx, &rel, symbio_plugin_api::VdfsRequest::Read)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .text
            .unwrap();
        assert_eq!(got, text);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 清理策略：按修改时间只保留最新 N 个存档。
#[tokio::test]
async fn prune_keeps_only_latest_files() {
    let dir = temp_dir("prune");
    let provider = provider_in(&dir);
    for i in 0..6u32 {
        let content = format!("content-{i}-{}", "x".repeat(64));
        archive_full_text(&content, 64, Some(&provider))
            .await
            .expect("写入");
        tokio::time::sleep(std::time::Duration::from_millis(4)).await;
    }
    let vctx = VdfsContext::new(());
    assert_eq!(list_count(&provider).await, 6, "6 个 ≤ keep(20) 时不应清理");
    prune_archive_dir(&provider, 3).await;
    assert_eq!(list_count(&provider).await, 3, "应只保留最新 3 个");
    // 最新内容（content-5）必须还在
    let items = provider
        .dispatch(
            &vctx,
            ARCHIVE_REL_DIR,
            symbio_plugin_api::VdfsRequest::List {},
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let mut found = false;
    for item in items {
        let rel = format!("{ARCHIVE_REL_DIR}/{}", item.node.name);
        let got = provider
            .dispatch(&vctx, &rel, symbio_plugin_api::VdfsRequest::Read)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .text
            .unwrap();
        if got == format!("content-5-{}", "x".repeat(64)) {
            found = true;
        }
    }
    assert!(found, "保留的应是最新写入的文件");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 存档只落在 provider 根下（会话作用域），地址是相对地址而非物理路径。
#[tokio::test]
async fn archive_lives_under_the_session_scope() {
    let dir = temp_dir("scope");
    let provider = provider_in(&dir);
    let g = guard_result(&"line\n".repeat(3000), 500, Some(&provider), None).await;
    assert!(g.truncated);
    let rel = g.archive_path.expect("有 provider 时应产出存档地址");
    assert!(
        rel.starts_with(&format!("{ARCHIVE_REL_DIR}/tool_")),
        "地址应是作用域内的相对地址: {rel}"
    );
    assert!(
        dir.join(&rel).is_file(),
        "文件应落在 provider 根下: {}",
        dir.join(&rel).display()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 有会话 id 时，占位符印会话地址面虚拟地址，取回端与写入端同源。
#[tokio::test]
async fn retrieval_address_uses_session_vaddr() {
    let dir = temp_dir("vaddr");
    let provider = provider_in(&dir);
    let g = guard_result(
        &"line\n".repeat(3000),
        500,
        Some(&provider),
        Some("sess-abc"),
    )
    .await;
    assert!(g.truncated);
    let rel = g.archive_path.expect("有 provider 时应产出存档地址");
    // 占位符取回地址 = 运行期根名 + session 挂载段 + 会话 id + 相对地址。
    // 根名经 descend_addr 从 vdfs 插件的静态声明取（测试注入 `.test-vdfs-root`），不写字面量。
    let session_mount = symbio_plugin_api::descend_addr("", symbio_plugin_api::PLUGIN_ID_SESSION);
    let vaddr = retrieval_address(&rel, Some("sess-abc"));
    assert_eq!(vaddr, format!("{session_mount}/sess-abc/{rel}"));
    assert!(
        g.text.contains(&vaddr),
        "占位符应印会话地址面虚拟地址: {}",
        g.text
    );
    // 无会话 id 时退回相对地址（宿主态按 workdir 解析）
    assert_eq!(retrieval_address(&rel, None), rel);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 没有工作区作用域 → 不存档，占位符如实说明（不再退回物理临时目录）。
#[tokio::test]
async fn no_scope_means_no_archive() {
    let g = guard_result(&"line\n".repeat(3000), 500, None, None).await;
    assert!(g.truncated);
    assert!(g.archive_path.is_none());
    assert!(g.text.contains("完整输出未存档（无可写作用域）"));
    assert!(!g.text.contains("vdfs_read"));
}
