//! L0：单次工具结果守卫
//!
//! 工具结果往往很长（shell 输出、文件全文、网页抓取…），单条即可撑爆上下文窗口。
//! 本模块在工具结果写进上下文**之前**按 token 预算裁剪：
//!
//! 1. 估算 token 数 ≤ 预算 → 原样返回；
//! 2. 否则：把全文存档到**会话存档目录**（`<本插件目录>/<safe_id>/tool_archives/`，
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
use std::time::SystemTime;

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
    let now = crate::symbio_core::now_ms();
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
/// `<本插件目录>/<safe_id>/tool_archives/`（跟随会话生命周期）；
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
#[path = "tool_result_guard.test.rs"]
mod tests;
