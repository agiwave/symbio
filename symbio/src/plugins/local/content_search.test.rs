//! content_search.rs 单元测试
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//!
//! 重点钉 T-113 的两件事（都是「结果看着像对的、其实不对」的形状病）：
//!
//! 1. **输出形状**——`content` 模式每个匹配行一条、行间单换行（不再有空行）。
//!    修复前 `UTF8Sink` 给的 `text` 含行终止符，`format!` 原样拼进结果再
//!    `join("\n")` ⇒ 输出成 `行\n\n行`。
//! 2. **分页与匹配行对齐**——`apply_line_pagination` 按 `result.lines()` 切，
//!    输出行不再含空行后，`head_limit=2` 必须恰好覆盖 2 个匹配行。
//!
//! 三种 `output_mode` 各一条形状判据（对齐 e2e `t67` 的①②③，但这里直接
//! 打 `search`，不经 LLM/CLI——单测拦形状、e2e 拦出口，两层各管各的）。

use super::*;
use std::path::Path;
use std::sync::Arc;

fn tool() -> ContentSearchTool {
    ContentSearchTool::new(Arc::new(SecurityPolicy::default()))
}

/// 两个文件共 5 个匹配行：a.rs（3 行）/ b.rs（2 行）/ c.rs 不含标记
async fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        dir.path().join("a.rs"),
        "const x1 = 'MARK';\nconst x2 = 'MARK';\nconst x3 = 'MARK';\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        dir.path().join("b.rs"),
        "const y1 = 'MARK';\nconst y2 = 'MARK';\n",
    )
    .await
    .unwrap();
    tokio::fs::write(dir.path().join("c.rs"), "const z = 1;\n")
        .await
        .unwrap();
    dir
}

async fn search_all(dir: &Path, output_mode: &str) -> String {
    tool()
        .search("MARK", dir, output_mode, None, None, true, 0, 0, false)
        .await
        .expect("搜索应成功")
}

// ==================== 输出形状（三种 output_mode 各一） ====================

/// `content` 模式：每匹配行一条 `<路径>:<行号>:<行内容>`，**行间单换行**——
/// 空行即回归（T-113 修复前 `UTF8Sink` 的 text 带终止符，行间成 `\n\n`）。
#[tokio::test]
async fn content_mode_has_one_line_per_match_without_blank_lines() {
    let dir = fixture().await;
    let out = search_all(dir.path(), "content").await;

    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 5, "5 个匹配行 ⇒ 恰 5 条输出（实际 {lines:?}）");
    assert!(
        lines.iter().all(|l| !l.trim().is_empty()),
        "输出里不许有空行（实际 {out:?}）"
    );
    assert!(!out.contains("\n\n"), "行间必须是单换行（实际 {out:?}）");
    // 形状：<路径>:<行号>:<内容>；匹配行正文里不带终止符残留
    for l in &lines {
        let rest = l.split_once("MARK").expect("每条输出应含匹配正文").0;
        assert!(
            rest.rfind(':').is_some(),
            "应为 `<路径>:<行号>:` 前缀（实际 {l:?}）"
        );
    }
    // 行号逐文件递增：a.rs 1..=3，b.rs 1..=2
    assert!(
        lines.iter().any(|l| l.contains("a.rs:1:")),
        "缺 a.rs:1（{out:?}）"
    );
    assert!(
        lines.iter().any(|l| l.contains("a.rs:3:")),
        "缺 a.rs:3（{out:?}）"
    );
    assert!(
        lines.iter().any(|l| l.contains("b.rs:2:")),
        "缺 b.rs:2（{out:?}）"
    );
    assert!(
        !out.contains("c.rs"),
        "不含标记的 c.rs 不该进结果（{out:?}）"
    );
}

/// `files_with_matches` 模式：每命中文件一条**纯路径**，不带行号与正文
#[tokio::test]
async fn files_mode_lists_pure_paths_one_per_file() {
    let dir = fixture().await;
    let out = search_all(dir.path(), "files_with_matches").await;

    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "两个命中文件 ⇒ 恰 2 行（实际 {out:?}）");
    assert!(
        lines.iter().all(|l| !l.contains("MARK")),
        "纯路径模式不该带正文（实际 {out:?}）"
    );
    assert!(
        lines
            .iter()
            .all(|l| l.ends_with("a.rs") || l.ends_with("b.rs")),
        "每条应是文件路径（实际 {out:?}）"
    );
}

/// `count` 模式：每命中文件一条 `<路径>:<匹配行数>`
#[tokio::test]
async fn count_mode_reports_per_file_match_counts() {
    let dir = fixture().await;
    let out = search_all(dir.path(), "count").await;

    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "两个命中文件 ⇒ 恰 2 行（实际 {out:?}）");
    assert!(
        lines.iter().any(|l| l.ends_with("a.rs:3")),
        "a.rs 应为 3 个匹配行（实际 {out:?}）"
    );
    assert!(
        lines.iter().any(|l| l.ends_with("b.rs:2")),
        "b.rs 应为 2 个匹配行（实际 {out:?}）"
    );
}

// ==================== 分页与匹配行对齐 ====================

/// 分页判据的核心反向用例：`head_limit=2` ⇒ 恰好 **2 个匹配行**。
/// 修复前输出含空行，`apply_line_pagination` 按**输出行**切 ⇒ 只剩 1 个匹配行。
#[tokio::test]
async fn head_limit_counts_match_lines_not_output_lines() {
    let dir = fixture().await;
    let out = search_all(dir.path(), "content").await;
    let paged = apply_line_pagination(out, 0, 2);

    let matches: Vec<&str> = paged.lines().filter(|l| l.contains("MARK")).collect();
    assert_eq!(
        matches.len(),
        2,
        "head_limit=2 ⇒ 恰 2 个匹配行（实际 {paged:?}）"
    );
    assert_eq!(
        paged.lines().count(),
        2,
        "输出行数也应为 2（不许夹空行，实际 {paged:?}）"
    );
}

/// `offset` 跳过前面的**匹配行**：offset=1 ⇒ 第 1 个匹配行（a.rs:1）被跳过
#[tokio::test]
async fn offset_skips_leading_match_lines() {
    let dir = fixture().await;
    let out = search_all(dir.path(), "content").await;
    let paged = apply_line_pagination(out, 1, 0);

    let lines: Vec<&str> = paged.lines().collect();
    assert_eq!(lines.len(), 4, "5 个匹配行跳过 1 ⇒ 剩 4（实际 {paged:?}）");
    assert!(
        !lines.iter().any(|l| l.contains("a.rs:1:")),
        "offset=1 应跳过第 1 个匹配行（实际 {paged:?}）"
    );
    assert!(
        lines.iter().any(|l| l.contains("a.rs:2:")),
        "第 2 个匹配行应在（实际 {paged:?}）"
    );
}

/// 组合：offset=1 + head_limit=2 ⇒ 恰好第 2、3 个匹配行
#[tokio::test]
async fn offset_and_head_limit_combine_on_match_lines() {
    let dir = fixture().await;
    let out = search_all(dir.path(), "content").await;
    let paged = apply_line_pagination(out, 1, 2);

    let matches: Vec<&str> = paged.lines().filter(|l| l.contains("MARK")).collect();
    assert_eq!(matches.len(), 2, "（实际 {paged:?}）");
    assert!(
        matches[0].contains("a.rs:2:"),
        "第一个应是 a.rs:2（{paged:?}）"
    );
    assert!(
        matches[1].contains("a.rs:3:"),
        "第二个应是 a.rs:3（{paged:?}）"
    );
    assert!(
        !paged.contains("b.rs"),
        "head_limit 截在 a.rs 内 ⇒ b.rs 不该出现（{paged:?}）"
    );
}

/// 平凡值：offset=0 且 head_limit=0 ⇒ 原样返回（不做任何切分）
#[test]
fn zero_pagination_returns_input_verbatim() {
    let input = "a.rs:1:MARK\nb.rs:1:MARK".to_string();
    assert_eq!(apply_line_pagination(input.clone(), 0, 0), input);
}

// ==================== 无匹配的形状 ====================

/// 无匹配 ⇒ 单行「未找到匹配。」（不是空串、不是错误）
#[tokio::test]
async fn no_match_reports_single_line() {
    let dir = fixture().await;
    let out = tool()
        .search(
            "NOMATCHHERE",
            dir.path(),
            "content",
            None,
            None,
            true,
            0,
            0,
            false,
        )
        .await
        .expect("搜索应成功");
    assert_eq!(out, "未找到匹配。");
}
