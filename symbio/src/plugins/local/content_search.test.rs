//! content_search.rs 单元测试
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//!
//! 重点钉 T-113 的两件事（都是「结果看着像对的、其实不对」的形状病）：
//!
//! 1. **输出形状**——`content` 模式每个匹配行一条、行间单换行（不再有空行）。
//!    修复前 `UTF8Sink` 给的 `text` 含行终止符，`format!` 原样拼进结果再
//!    `join("\n")` ⇒ 输出成 `行\n\n行`。
//! 2. **分页窗口按匹配行对齐，且截断处给出读数**——`offset` / `head_limit` 切在
//!    **收集**阶段（不是先收满再切），所以窗口越过 `MAX_RESULTS` 也拿得到后续段落；
//!    没给全时尾部一条读数说清总量、已给区间与下一个 `offset`。
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
        .search(
            "MARK",
            dir,
            output_mode,
            None,
            None,
            true,
            0,
            0,
            false,
            0,
            0,
        )
        .await
        .expect("搜索应成功")
}

/// 带分页窗口的搜索：`offset` / `head_limit` 现在切在收集阶段
async fn search_window(dir: &Path, offset: usize, head_limit: usize) -> String {
    tool()
        .search(
            "MARK", dir, "content", None, None, true, 0, 0, false, offset, head_limit,
        )
        .await
        .expect("分页搜索应成功")
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

/// 输出序是**确定的**：文件按名有序、文件内按行号升序。
/// 遍历序不钉住时，NTFS 给名字序而 ext4 给哈希序 ⇒ 同一查询行序不同，
/// 下面两条分页用例（`offset` / `head_limit` 的窗口）在另一平台上就变成另一份答案。
#[tokio::test]
async fn output_order_is_name_then_line_not_fs_walk_order() {
    let dir = fixture().await;
    let lines: Vec<String> = search_all(dir.path(), "content")
        .await
        .lines()
        .map(str::to_string)
        .collect();

    let at = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("缺 {needle}（{lines:?}）"))
    };
    assert!(
        at("a.rs:1:") < at("a.rs:2:") && at("a.rs:2:") < at("a.rs:3:"),
        "同一文件内按行号升序（{lines:?}）"
    );
    assert!(
        at("a.rs:3:") < at("b.rs:1:") && at("b.rs:1:") < at("b.rs:2:"),
        "a.rs 的 3 行整体先于 b.rs，且 b.rs 内升序（{lines:?}）"
    );
}

/// 分页判据的核心反向用例：`head_limit=2` ⇒ 恰好 **2 个匹配行**（外加分页读数一行）。
/// 修复前输出含空行，分页按**输出行**切 ⇒ 只剩 1 个匹配行。
#[tokio::test]
async fn head_limit_counts_match_lines_not_output_lines() {
    let dir = fixture().await;
    let paged = search_window(dir.path(), 0, 2).await;

    let matches: Vec<&str> = paged.lines().filter(|l| l.contains("MARK")).collect();
    assert_eq!(
        matches.len(),
        2,
        "head_limit=2 ⇒ 恰 2 个匹配行（实际 {paged:?}）"
    );
    let lines: Vec<&str> = paged.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "2 个匹配行 + 1 行分页读数（实际 {paged:?}）"
    );
    assert!(
        lines[2].starts_with("[结果已分页"),
        "末行必须是读数（{lines:?}）"
    );
}

/// `offset` 跳过前面的**匹配行**：offset=1 ⇒ 第 1 个匹配行（a.rs:1）被跳过。
/// 跳过第 1 行后 4 行全部给完（窗口未再收紧）⇒ 没有隐藏段落 ⇒ 不带读数。
#[tokio::test]
async fn offset_skips_leading_match_lines() {
    let dir = fixture().await;
    let paged = search_window(dir.path(), 1, 0).await;

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
    assert!(
        !paged.contains("[结果已分页"),
        "全给完了就不该喊截断（实际 {paged:?}）"
    );
}

/// 组合：offset=1 + head_limit=2 ⇒ 恰好第 2、3 个匹配行
#[tokio::test]
async fn offset_and_head_limit_combine_on_match_lines() {
    let dir = fixture().await;
    let paged = search_window(dir.path(), 1, 2).await;

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

/// 窗口切满但未给全 ⇒ 尾部读数必须同时给出**总量**与**取回方式**
/// （T-140：只说「限制了」不说「一共多少」，聚合型问题就逼出原样重发）。
#[tokio::test]
async fn truncated_tail_reports_total_and_next_offset() {
    let dir = fixture().await;
    let out = search_window(dir.path(), 0, 2).await;
    let note = out
        .lines()
        .last()
        .expect("截断结果应以读数结尾")
        .to_string();

    assert!(note.contains("共 5 处命中"), "读数缺真实命中总数（{note}）");
    assert!(note.contains("5 行输出"), "读数缺输出总行数（{note}）");
    assert!(note.contains("第 1–2 行"), "读数缺已给区间（{note}）");
    assert!(note.contains("还有 3 行"), "读数缺未给条数（{note}）");
    assert!(note.contains("offset=2"), "读数缺下一页抓手（{note}）");
    assert!(
        note.contains("output_mode=\"count\""),
        "读数缺「只要总数」的替代口径（{note}）"
    );
}

/// 翻页真的能翻到窗口之外的段落：offset=2 拿到第 3、4 个匹配行——
/// 修复前收集在上限处 `break`，`offset` 越界只会翻回同一份被截断的结果。
#[tokio::test]
async fn paging_reaches_lines_beyond_the_first_window() {
    let dir = fixture().await;
    let page2 = search_window(dir.path(), 2, 2).await;
    let matches: Vec<&str> = page2.lines().filter(|l| l.contains("MARK")).collect();
    assert_eq!(matches.len(), 2, "第二页应有 2 个匹配行（{page2:?}）");
    assert!(matches[0].contains("a.rs:3:"), "{matches:?}");
    assert!(matches[1].contains("b.rs:1:"), "{matches:?}");
    // 读数口径跨页一致：总数不随窗口变化
    assert!(page2
        .lines()
        .last()
        .expect("仍有未给段落")
        .contains("共 5 处命中"));
}

/// 窗口越过末尾：说「越界」而不是「未找到匹配」——后者会把模型推向重发同一条查询。
#[tokio::test]
async fn window_past_end_says_so_instead_of_no_match() {
    let dir = fixture().await;
    let out = search_window(dir.path(), 99, 0).await;
    assert!(out.starts_with("[窗口越过末尾"), "实际 {out:?}");
    assert!(out.contains("共 5 处命中"), "越界读数也要给总数（{out}）");
    assert!(!out.contains("未找到匹配"), "有命中时不许说未找到（{out}）");
}

/// 平凡值：offset=0 且 head_limit=0 ⇒ 全量给完、不加读数（结果本身未被限制）
#[tokio::test]
async fn no_window_returns_everything_without_note() {
    let dir = fixture().await;
    let out = search_all(dir.path(), "content").await;
    assert_eq!(out.lines().count(), 5, "5 个匹配行全给（实际 {out:?}）");
    assert!(!out.contains('['), "未截断不加读数（实际 {out:?}）");
}

// ==================== 结果形状契约（T-196 / 04 报告建议 3） ====================

/// 结果形状契约：正文写在 `output` 字符串里（不是整包 JSON、不是别的键）。
///
/// 消费方 `extract_result`（loop 插件）第 2 条判据读 `output`；两侧互不可见，
/// 这条断言是「约定可执行」的生产方一半。前面那些用例打的是 `search` 内部方法
/// （只验正文形状），这条走 `Capability::execute`，钉的是**交付给编排层的信封形状**。
#[tokio::test]
async fn result_carries_the_llm_visible_text_in_output() {
    use crate::symbio_core::{ExecEnv, PluginInvokeRequest, PluginSimpleRequest, WORKDIR};

    let dir = fixture().await;
    let req = PluginSimpleRequest::new(None, None);
    req.set(WORKDIR, dir.path().to_string_lossy().to_string());
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(req);
    let env = ExecEnv::from_request(&*ctx);

    let data = tool()
        .execute(
            serde_json::json!({ "pattern": "MARK", "path": "." }),
            &env,
            ctx,
        )
        .await
        .expect("搜索应成功");

    let out = data["output"]
        .as_str()
        .expect("正文必须在 output 字符串里（extract_result 第 2 条判据）");
    assert!(out.contains("a.rs:"), "正文应带命中行（实际: {out:?}）");
    assert_eq!(data["truncated"], serde_json::json!(false));
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
            0,
            0,
        )
        .await
        .expect("搜索应成功");
    assert_eq!(out, "未找到匹配。");
}
