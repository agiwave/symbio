//! `tool_result_guard` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `tool_result_guard.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// 测试用会话存储根（只算路径 / 落临时目录，不碰真实 homedir）
fn root() -> std::path::PathBuf {
    // 末段保留 `session`：下条用例断言存档路径含 `session/<id>/tool_archives`
    std::env::temp_dir()
        .join("symbio-guard-test")
        .join("session")
}

#[test]
fn short_text_passes_through() {
    let g = guard_tool_result("hello world", 8192, None, &root());
    assert!(!g.truncated);
    assert_eq!(g.text, "hello world");
}

#[test]
fn long_text_is_truncated_and_keeps_head_tail() {
    // 构造远超预算的多行文本
    let line = "fn main() {\n    println!(\"line\");\n}\n";
    let big = line.repeat(2000);
    let g = guard_tool_result(&big, 500, None, &root());
    assert!(g.truncated);
    assert!(g.text.contains("[... 已省略"));
    // 统一取回协议：占位符含存档路径与 vdfs_read 取回入口（P1-2）
    assert!(g.text.contains("完整输出已存档至: "));
    assert!(g.text.contains("vdfs_read"));
    // head 与 tail 应分别保留原文片段
    assert!(g.text.contains("fn main()"));
    // 文本显著缩短（预算 500 token ≈ 文本远小于原文）
    assert!(g.text.len() < big.len() / 2);
}

/// 单行超长内容：按行截断失效（首行整行放行），应按字符截断行首
/// （真实会话实证：12k token 单行 glob 结果只省略 1/3，压缩近乎失效）
#[test]
fn single_long_line_is_char_truncated() {
    let huge = format!("{{\"entries\":[\"{}\"]}}", "x".repeat(10_000));
    let g = guard_tool_result(&huge, 300, None, &root());
    assert!(g.truncated, "单行超预算应触发存档截断");
    assert!(g.text.contains("vdfs_read"), "占位提示应存在");
    assert!(
        g.text.starts_with("{\"entries"),
        "行首应保留: {}",
        &g.text[..40.min(g.text.len())]
    );
}

fn test_archive_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("symbio_guard_tests")
        .join(format!("{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// P1-1 防碰撞：同毫秒同 token 数的不同内容必须落到不同文件
/// （旧实现文件名 = 秒级时间戳 + token 数，同秒同 token 数互相覆盖）。
#[test]
fn same_milli_same_tokens_do_not_collide() {
    let dir = test_archive_dir("collide");
    // 同长度（≈同 token 数）但内容不同，毫秒内连续写入
    let a = "a".repeat(4096);
    let b = format!("a{}a", "b".repeat(4094));
    let p1 = archive_into_dir(&a, 1024, &dir).expect("写入 a");
    let p2 = archive_into_dir(&b, 1024, &dir).expect("写入 b");
    assert_ne!(p1, p2, "同毫秒同 token 数的不同内容不得共享同一文件名");
    assert_eq!(std::fs::read_to_string(&p1).unwrap(), a);
    assert_eq!(std::fs::read_to_string(&p2).unwrap(), b);
    let _ = std::fs::remove_dir_all(&dir);
}

/// P1-1 清理策略：目录按修改时间只保留最新 N 个存档。
#[test]
fn prune_keeps_only_latest_files() {
    let dir = test_archive_dir("prune");
    for i in 0..6u32 {
        let content = format!("content-{i}-{}", "x".repeat(64));
        archive_into_dir(&content, 64, &dir).expect("写入");
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        6,
        "6 个 ≤ keep(20) 时不应清理"
    );
    prune_archive_dir(&dir, 3);
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        3,
        "应只保留最新 3 个"
    );
    // 最新内容（content-5）必须还在
    let newest = format!("content-5-{}", "x".repeat(64));
    assert!(
        std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| std::fs::read_to_string(e.path()).unwrap() == newest),
        "保留的应是最新写入的文件"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// P1-1 存档位置：会话标识可用时，存档应落在会话目录 tool_archives/ 下
///（跟随会话生命周期），而非系统临时目录。
#[test]
fn archive_prefers_session_dir_when_session_id_given() {
    let sid = "test_session_guard_01";
    let g = guard_tool_result(&"line\n".repeat(3000), 500, Some(sid), &root());
    assert!(g.truncated);
    let p = g.archive_path.expect("会话目录可写时应产出存档路径");
    let expected_frag = format!(
        "session{}{}{}tool_archives",
        std::path::MAIN_SEPARATOR,
        sid,
        std::path::MAIN_SEPARATOR
    );
    assert!(
        p.contains(&expected_frag),
        "存档应位于会话 tool_archives/ 目录: {p}"
    );
    // 收尾：删除该测试会话的存档目录（不影响其他测试）
    let dir = root().join(sid).join("tool_archives");
    let _ = std::fs::remove_dir_all(dir);
}
