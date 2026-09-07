//! L0：单次工具结果守卫
//!
//! 工具结果往往很长（shell 输出、文件全文、网页抓取…），单条即可撑爆上下文窗口。
//! 本模块在工具结果写进上下文**之前**按 token 预算裁剪：
//!
//! 1. 估算 token 数 ≤ 预算 → 原样返回；
//! 2. 否则：把全文存档到临时目录（可经 `local/file_read` 取回），并对正文做
//!    **head/tail 摘要**（保留前 60% + 后 40% 预算，尾部通常是结论/错误，更关键），
//!    中间插入占位说明。
//!
//! 与既有压缩层（L1/L2/L3）的边界：本层只处理**单条**结果，是"语义上限"；
//! 物理字节上限（shell/fetch 1MB 等）是最后一道防线，二者不冲突。

use crate::symbio_core::{default_tokenizer, Tokenizer};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 单条工具结果 token 预算（默认 ≈ 32KB 文本），可由调用方按需覆盖。
pub const DEFAULT_TOOL_RESULT_TOKEN_CAP: usize = 8192;

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

/// 把超长文本按预算裁剪成 head/tail 摘要。
///
/// 优先按**行**截断（结构化输出友好，不会切断 JSON/表格行）；单行超长时退化为按字符。
fn split_head_tail(text: &str, head_budget: usize, tail_budget: usize) -> (String, String) {
    let tok = default_tokenizer();
    let lines: Vec<&str> = text.lines().collect();

    // head：从前往后贪心累加，直到超出 head_budget
    let mut head = String::new();
    let mut used = 0usize;
    let mut head_line_count = 0usize;
    for &line in &lines {
        let c = tok.count(line) + tok.count("\n");
        if used + c > head_budget && !head.is_empty() {
            break;
        }
        used += c;
        head.push_str(line);
        head.push('\n');
        head_line_count += 1;
    }

    // tail：从尾部（跳过已被 head 取走的部分）往回贪心累加
    let mut tail_lines: Vec<&str> = Vec::new();
    let mut used_t = 0usize;
    for &line in lines[head_line_count.min(lines.len())..].iter().rev() {
        let c = tok.count(line) + tok.count("\n");
        if used_t + c > tail_budget && !tail_lines.is_empty() {
            break;
        }
        used_t += c;
        tail_lines.push(line);
    }
    tail_lines.reverse();
    let tail = tail_lines.join("\n");

    (head, tail)
}

/// 把全文写入临时目录存档（best-effort）。失败返回 None，不阻断主流程。
fn archive_full_text(text: &str, token_count: usize) -> Option<String> {
    let dir = std::env::temp_dir().join("symbio_tool_archives");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_name = format!("tool_{}_{}.txt", ts, token_count);
    let path: PathBuf = dir.join(file_name);
    match std::fs::write(&path, text) {
        Ok(_) => path.to_str().map(|s| s.to_string()),
        Err(_) => None,
    }
}

/// 守卫工具结果：超过预算则存档 + head/tail 摘要，否则原样返回。
pub fn guard_tool_result(text: &str, budget_tokens: usize) -> GuardedResult {
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

    let archive_path = archive_full_text(text, n);

    let head_budget = ((budget_tokens as f64) * 0.6) as usize;
    let tail_budget = budget_tokens.saturating_sub(head_budget);
    let (head, tail) = split_head_tail(text, head_budget, tail_budget);

    let omit = n.saturating_sub(budget_tokens);
    let archive_hint = match &archive_path {
        Some(p) => format!("完整输出已存档：{p}（可用 local/file_read 按 offset/limit 分段读取）"),
        None => "完整输出未存档（临时目录不可写）".to_string(),
    };
    let placeholder = format!(
        "\n[... 已省略约 {omit} tokens 的中间内容。{archive_hint} ...]\n"
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
mod tests {
    use super::*;

    #[test]
    fn short_text_passes_through() {
        let g = guard_tool_result("hello world", 8192);
        assert!(!g.truncated);
        assert_eq!(g.text, "hello world");
    }

    #[test]
    fn long_text_is_truncated_and_keeps_head_tail() {
        // 构造远超预算的多行文本
        let line = "fn main() {\n    println!(\"line\");\n}\n";
        let big = line.repeat(2000);
        let g = guard_tool_result(&big, 500);
        assert!(g.truncated);
        assert!(g.text.contains("[... 已省略"));
        // head 与 tail 应分别保留原文片段
        assert!(g.text.contains("fn main()"));
        // 文本显著缩短（预算 500 token ≈ 文本远小于原文）
        assert!(g.text.len() < big.len() / 2);
    }
}
