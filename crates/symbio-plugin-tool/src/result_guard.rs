//! L0：单次工具结果守卫（执行体侧）
//!
//! 工具结果往往很长（shell 输出、文件全文、网页抓取…），单条即可撑爆上下文窗口。
//! 本模块在结果交回环**之前**按 token 预算裁剪：
//!
//! 1. 估算 token 数 ≤ 预算 → 原样返回；
//! 2. 否则：把全文存档到**会话存档目录**（由调用方在载荷里给出的
//!    `archive_dir`，可经 `local/file_read` 取回），并对正文做
//!    **head/tail 摘要**（保留前 60% + 后 40% 预算，尾部通常是结论/错误，更关键），
//!    中间插入占位说明。
//!
//! 存档位置跟随会话目录持久化（历史提示不会因临时目录被清理而变成死链）。
//! 文件名 = 毫秒时间戳 + token 数 + 内容 FNV 指纹，杜绝「同秒同 token 数互相覆盖」
//! 的碰撞；目录按修改时间保留最新 [`ARCHIVE_KEEP`] 个文件，防止无限累积。
//!
//! 计量与头尾切分机制住在 `symbio_plugin_api::text`（环与世界共用同一套预算算术）；
//! 本模块只管**工具结果这一种策略**：触发条件、60/40 配比、占位符文案与存档布局。

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use symbio_plugin_api::clock_now_ms;
use symbio_plugin_api::text::{default_tokenizer, split_head_tail, HeadTailSplit, Tokenizer};

/// 单条工具结果 token 预算（默认 ≈ 32KB 文本），可由调用方按需覆盖。
pub const DEFAULT_TOKEN_CAP: usize = 8192;

/// 每个会话的 L0 存档目录保留的最新文件数（防无限累积；超出按修改时间淘汰最旧）。
pub const ARCHIVE_KEEP: usize = 20;

/// 取回指引文案（三层压缩占位符共用同一格式；随驱动器走，因为写占位符的是这里）。
///
/// 与 `session/paths.rs` 的取回协议同义——它描述的是**用户可见的取回入口**，
/// 不是会话存储布局；跟着写文案的一侧住，避免环为了一个字符串去认识会话。
pub const RETRIEVAL_HINT: &str = "（取回：vdfs_read 该路径，按 offset/limit 分段读取）";

/// 守卫结果
#[derive(Default)]
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

/// L0 存档目录：会话子目录可用时用它（跟随会话生命周期，可被历史取回），
/// 否则退回系统临时目录兜底（仅作为目录不可写时的降级路径）。
fn resolve_archive_dir(archive_dir: &Path) -> Option<PathBuf> {
    if std::fs::create_dir_all(archive_dir).is_ok() {
        return Some(archive_dir.to_path_buf());
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
/// 文件名 = `tool_{毫秒ts}_{token数}_{内容指纹}.txt`：毫秒 + 指纹双保险。
fn archive_full_text(text: &str, token_count: usize, archive_dir: &Path) -> Option<String> {
    let dir = resolve_archive_dir(archive_dir)?;
    archive_into_dir(text, token_count, &dir)
}

/// 写入指定目录并执行清理（目录由调用方解析；拆出便于测试注入目录）。
fn archive_into_dir(text: &str, token_count: usize, dir: &std::path::Path) -> Option<String> {
    let now = clock_now_ms();
    let file_name = format!(
        "tool_{}_{}_{}.txt",
        now,
        token_count,
        content_fingerprint(text)
    );
    let path: PathBuf = dir.join(file_name);
    match std::fs::write(&path, text) {
        Ok(_) => {
            prune_archive_dir(dir, ARCHIVE_KEEP);
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

/// 占位符前缀：omit 统计口径保持一致。
fn omit_placeholder_prefix(omit: usize) -> String {
    format!("\n[... 已省略约 {omit} tokens 的中间内容。")
}

/// 守卫工具结果：超过预算则存档 + head/tail 摘要，否则原样返回。
///
/// `archive_dir`：会话的 L0 存档目录（环在载荷里给出）。
/// 目录不可写时退回系统临时目录兜底。
pub fn guard_result(text: &str, budget_tokens: usize, archive_dir: &Path) -> GuardedResult {
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

    let archive_path = archive_full_text(text, n, archive_dir);

    let omit = n.saturating_sub(budget_tokens);
    // 统一取回协议：占位符含存档路径与 vdfs_read 取回入口。
    let archive_hint = match &archive_path {
        Some(p) => format!("完整输出已存档至: {p}{RETRIEVAL_HINT}"),
        None => "完整输出未存档（存档目录不可写）".to_string(),
    };
    let placeholder = format!("{} {archive_hint} ...]", omit_placeholder_prefix(omit));

    let HeadTailSplit { head, tail } = split_head_tail(
        text,
        ((budget_tokens as f64) * 0.6) as usize,
        budget_tokens - ((budget_tokens as f64) * 0.6) as usize,
        tok,
    );

    let mut out = String::with_capacity(head.len() + tail.len() + placeholder.len());
    out.push_str(&head);
    out.push_str(&placeholder);
    out.push_str(&tail);

    GuardedResult {
        text: out,
        truncated: true,
        original_tokens: n,
        archive_path,
    }
}

#[cfg(test)]
#[path = "result_guard.test.rs"]
mod tests;
