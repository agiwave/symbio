//! L0：单次工具结果守卫
//!
//! 工具结果往往很长（shell 输出、文件全文、网页抓取…），单条即可撑爆上下文窗口。
//! 本模块在工具结果写进上下文**之前**按 token 预算裁剪：
//!
//! 1. 估算 token 数 ≤ 预算 → 原样返回；
//! 2. 否则：把全文存档到**会话存档目录**（`<homedir>/plugins/session/<safe_id>/tool_archives/`，
//!    可经 `local/file_read` 取回），并对正文做
//!    **head/tail 摘要**（保留前 60% + 后 40% 预算，尾部通常是结论/错误，更关键），
//!    中间插入占位说明。
//!
//! 存档位置设计（P1-1）：工具结果语义上是会话资产，历史上写入系统临时目录会被
//! OS 清理造成历史提示死链；现跟随会话目录持久化。文件名 = 毫秒时间戳 + token 数 + 内容 FNV 指纹，杜绝旧实现"同秒同 token 数互相覆盖"的碰撞；目录按修改时间
//! 保留最新 [`TOOL_ARCHIVE_KEEP`] 个文件，防止无限累积。
//!
//! 与既有压缩层（L2 自动摘要 / L3 请求视图骨架化）的边界：本层只处理**单条**结果，是"语义上限"；
//! 物理字节上限（shell/fetch 1MB 等）是最后一道防线，二者不冲突。

use super::text_split::{split_head_tail, HeadTailSplit};
use super::tokenizer::{default_tokenizer, Tokenizer};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 单条工具结果 token 预算（默认 ≈ 32KB 文本），可由调用方按需覆盖。
pub const DEFAULT_TOOL_RESULT_TOKEN_CAP: usize = 8192;

/// 每个会话的 L0 存档目录保留的最新文件数（防无限累积；超出按修改时间淘汰最旧）。
pub const TOOL_ARCHIVE_KEEP: usize = 20;

/// 守卫结果
pub struct GuardedResult {
    /// 裁剪后的文本（已可安全放进上下文）
    pub text: String,
    /// 是否被截断/存档
    pub truncated: bool,
    /// 原始 token 估算
    pub original_tokens: usize,
    /// 存档路径（若写入成功）
    pub archive_path: Option<String>,
}

/// L0 存档目录：会话标识可用时存入会话目录（跟随会话生命周期，可被历史取回），
/// 否则退回系统临时目录兜底（与旧行为一致，仅作为无会话上下文时的降级路径）。
fn resolve_archive_dir(session_id: Option<&str>) -> Option<PathBuf> {
    if let Some(sid) = session_id {
        if !sid.trim().is_empty() {
            // 路径派生统一走 paths 模块（safe_id / 会话根目录的唯一权威实现）
            let dir = super::paths::session_subdir(sid, super::paths::TOOL_ARCHIVES_SUBDIR);
            if std::fs::create_dir_all(&dir).is_ok() {
                return Some(dir);
            }
        }
    }
    let fallback = std::env::temp_dir().join("symbio_tool_archives");
    std::fs::create_dir_all(&fallback).ok()?;
    Some(fallback)
}

/// 内容 FNV-1a 64 位指纹（取高 32 位十六进制）。
///
/// 同秒同 token 数的两个不同结果，指纹几乎必然不同 → 文件名不碰撞；
/// 完全相同的内容共享同一指纹 → 覆盖无害（内容一致）。
fn content_fingerprint(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (h >> 32) as u32)
}

/// 把全文写入存档目录（best-effort）。失败返回 None，不阻断主流程。
///
/// 文件名 = `tool_{毫秒ts}_{token数}_{内容指纹}.txt`：毫秒 + 指纹双保险，
/// 修复旧实现"秒级时间戳 + token 数"在同秒同 token 数时互相覆盖的碰撞缺陷。
/// 写入成功后按修改时间保留最新 [`TOOL_ARCHIVE_KEEP`] 个存档。
fn archive_full_text(text: &str, token_count: usize, session_id: Option<&str>) -> Option<String> {
    let dir = resolve_archive_dir(session_id)?;
    archive_into_dir(text, token_count, &dir)
}

/// 写入指定目录并执行清理（目录由调用方解析；拆出便于测试注入目录）。
fn archive_into_dir(text: &str, token_count: usize, dir: &std::path::Path) -> Option<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let file_name = format!(
        "tool_{}_{}_{}.txt",
        now,
        token_count,
        content_fingerprint(text)
    );
    let path: PathBuf = dir.join(file_name);
    match std::fs::write(&path, text) {
        Ok(_) => {
            prune_archive_dir(dir, TOOL_ARCHIVE_KEEP);
            path.to_str().map(|s| s.to_string())
        }
        Err(_) => None,
    }
}

/// 按修改时间保留最新 `keep` 个存档文件，淘汰更旧的（best-effort，失败静默）。
fn prune_archive_dir(dir: &std::path::Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, e.path()))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by_key(|(t, _)| std::cmp::Reverse(*t)); // 新 → 旧
    for (_, path) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
}

/// guard / summarize 共用的 head/tail 摘要拼装核心。
///
/// 两者唯一的差异是占位符文案（guard 附带存档取回指引，summarize 提示重跑工具），
/// 预算切分（60%/40%）、omit 计算、三段拼接逻辑完全一致 —— 收敛于此，防止漂移。
/// 切分机制本体在 `text_split::split_head_tail`（头偏 60/40 的策略在此处选择）。
fn assemble_head_tail_summary(text: &str, budget_tokens: usize, placeholder: String) -> String {
    let head_budget = ((budget_tokens as f64) * 0.6) as usize;
    let tail_budget = budget_tokens.saturating_sub(head_budget);
    let HeadTailSplit { head, tail } = split_head_tail(
        text,
        head_budget,
        tail_budget,
        super::tokenizer::default_tokenizer(),
    );

    let mut out = String::with_capacity(head.len() + tail.len() + placeholder.len());
    out.push_str(&head);
    out.push_str(&placeholder);
    out.push_str(&tail);
    out
}

/// guard / summarize 共用的占位符前缀：omit 统计口径保持一致。
fn omit_placeholder_prefix(omit: usize) -> String {
    format!("\n[... 已省略约 {omit} tokens 的中间内容。")
}

/// 守卫工具结果：超过预算则存档 + head/tail 摘要，否则原样返回。
///
/// `session_id`：当前会话标识。提供时存档写入会话目录
/// `<homedir>/plugins/session/<safe_id>/tool_archives/`（跟随会话生命周期）；
/// 为 None/空或目录创建失败时退回系统临时目录兜底。
pub fn guard_tool_result(
    text: &str,
    budget_tokens: usize,
    session_id: Option<&str>,
) -> GuardedResult {
    let tok = default_tokenizer();
    let n = tok.count(text);
    if n <= budget_tokens {
        return GuardedResult {
            text: text.to_string(),
            truncated: false,
            original_tokens: n,
            archive_path: None,
        };
    }

    let archive_path = archive_full_text(text, n, session_id);

    let omit = n.saturating_sub(budget_tokens);
    // 统一取回协议（P1-2）：三层压缩占位符共用同一格式 —— 「已存档至: <路径> +
    // 统一取回入口 local/file_read + 统一分段参数 offset/limit」。
    // 取回指引文案唯一来源：paths::RETRIEVAL_HINT。
    let archive_hint = match &archive_path {
        Some(p) => format!("完整输出已存档至: {p}{}", super::paths::RETRIEVAL_HINT),
        None => "完整输出未存档（存档目录不可写）".to_string(),
    };
    let placeholder = format!("{} {archive_hint} ...]", omit_placeholder_prefix(omit));

    let out = assemble_head_tail_summary(text, budget_tokens, placeholder);

    GuardedResult {
        text: out,
        truncated: true,
        original_tokens: n,
        archive_path,
    }
}

/// 请求视图级摘要：与 [`guard_tool_result`] 相同的 head/tail 摘要，但**不写存档文件**。
///
/// 用于请求视图层的轮次淡化（fade）：存储层始终保留全文，淡化只作用于每次请求的
/// 视图副本，无需持久化副本；模型如需完整输出可重新运行对应工具。
pub fn summarize_tool_result(text: &str, budget_tokens: usize) -> String {
    summarize_head_tail(
        text,
        budget_tokens,
        "历史轮次结果已淡化以控制上下文长度，如需完整输出请重新运行该工具。",
    )
}

/// 请求视图级 head/tail 摘要（**无存档**）：token 超预算时保留头尾、中段以
/// `omit_note` 占位。工具结果淡化与内容节点淡化共用的机制本体。
pub(crate) fn summarize_head_tail(text: &str, budget_tokens: usize, omit_note: &str) -> String {
    let tok = default_tokenizer();
    let n = tok.count(text);
    if n <= budget_tokens {
        return text.to_string();
    }

    let omit = n.saturating_sub(budget_tokens);
    let placeholder = format!("{} {omit_note} ...]", omit_placeholder_prefix(omit));

    assemble_head_tail_summary(text, budget_tokens, placeholder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_passes_through() {
        let g = guard_tool_result("hello world", 8192, None);
        assert!(!g.truncated);
        assert_eq!(g.text, "hello world");
    }

    #[test]
    fn long_text_is_truncated_and_keeps_head_tail() {
        // 构造远超预算的多行文本
        let line = "fn main() {\n    println!(\"line\");\n}\n";
        let big = line.repeat(2000);
        let g = guard_tool_result(&big, 500, None);
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
        let g = guard_tool_result(&huge, 300, None);
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
        let g = guard_tool_result(&"line\n".repeat(3000), 500, Some(sid));
        assert!(g.truncated);
        let p = g.archive_path.expect("会话目录可写时应产出存档路径");
        let expected_frag = format!(
            "plugins{}session{}{}{}tool_archives",
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR,
            sid,
            std::path::MAIN_SEPARATOR
        );
        assert!(
            p.contains(&expected_frag),
            "存档应位于会话 tool_archives/ 目录: {p}"
        );
        // 收尾：删除该测试会话的存档目录（不影响其他测试）
        let dir = crate::symbio_core::HomedirRegistry::get()
            .join("plugins")
            .join("session")
            .join(sid)
            .join("tool_archives");
        let _ = std::fs::remove_dir_all(dir);
    }
}
