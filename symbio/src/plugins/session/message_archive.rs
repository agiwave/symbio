//! 会话消息内容压缩模块
//!
//! 压缩策略（针对单条消息）：
//! - ToolCall 参数节点**永久豁免**（硬约束）：ToolCall 自身 content 携带请求参数
//!   JSON，骨架化会让模型误读"自己上次执行了参数为存档占位符的 edit"；
//!   参数超预算的治理属请求视图层（skeletonize_toolcall），此处不碰。
//! - 内容超阈值（行数超过阈值或单条 token 超预算）时：
//!   1. 将完整内容写入会话目录下的存档文件
//!   2. 保留头部 1/4 + 其余尾部的行（对齐 L0 split_head_tail 策略，
//!      首行常含结论/路径/计划骨架，只保尾部会把它挤出模型视野）
//!   3. 在保留内容头部追加系统注释，标明存档路径与统一取回方式
//! - 不足阈值的消息原样返回
//!
//! 体检备注（audit-5）：原文件名 `compress.rs` 与 `compression.rs`
//! （上下文语义压缩服务）命名易混——本模块职责是单条消息的
//! "物理脱水存档 + 还原"，与上下文语义压缩无关，故更名。

use super::tokenizer::Tokenizer;
use super::types::ChatMessage;
use crate::symbio_core::schemas::session::chat_message::{MessageContent, MessageType};
use crate::symbio_core::PluginError;
use std::path::Path;

/// 存档子目录名（位于会话目录内；规范值由 [`super::paths::MESSAGES_SUBDIR`] 定义，
/// 此处保留 re-export 以兼容既有调用方，避免双份常量漂移）。
pub const MESSAGES_SUBDIR: &str = super::paths::MESSAGES_SUBDIR;

/// 单条消息 token 预算：行数阈值之外的第二触发条件。
/// 单行长 JSON/URL/base64（1 行但数万 token）必须触发压缩，否则绕过防线。
pub const MESSAGE_TOKEN_CAP: usize = 2048;

/// 压缩消息的标识前缀（使用类似注释的样式，避免干扰主视觉）
pub const COMPRESS_PREFIX: &str = "<!-- [内容已压缩] -->";

/// 对单条消息进行内容压缩。
///
/// - `archive_rel_path`: 存档文件相对于会话目录的路径（用于实际写入，如 "messages/msg_1.txt"）。
/// - `archive_display_path`: 在消息中显示的存档文件路径（通常是相对于工作区根目录的路径，便于 LLM 读取）。
/// - 若消息内容已经包含 `COMPRESS_PREFIX`，跳过（防止重复压缩）。
/// - 若消息内容行数 <= `threshold`，不压缩，返回 None。
/// - 否则将完整内容写入 `session_dir/archive_rel_path`，
///   并返回截断后的消息。
pub async fn compress_message(
    session_dir: &Path,
    msg: &ChatMessage,
    threshold: usize,
    archive_rel_path: &str,
    archive_display_path: &str,
) -> Result<Option<ChatMessage>, PluginError> {
    // 取出文本内容，非文本消息跳过
    let full_text = match &msg.content {
        Some(MessageContent::Text(s)) => s.clone(),
        Some(MessageContent::Parts(_)) => {
            // Parts 类型先转成文本再判断
            msg.content
                .as_ref()
                .map(|c| c.to_text())
                .unwrap_or_default()
        }
        None => return Ok(None),
    };

    // 1. 检查是否已经压缩过
    if full_text.trim_start().starts_with(COMPRESS_PREFIX) {
        return Ok(None);
    }

    // 1.5 ToolCall 参数永久豁免：参数 JSON 一旦骨架化，请求视图里会呈现
    //     "上次执行了参数为存档占位符的工具调用"，模型据此误记自身行为。
    if msg.msg_type == Some(MessageType::ToolCall) {
        return Ok(None);
    }

    let lines: Vec<&str> = full_text.lines().collect();

    // 2. 触发条件：行数超阈值 **或** 单条消息 token 超预算。
    //    仅按行数判断会让单行长 JSON/URL/base64 完全绕过压缩
    //    （1 行 ≤ 阈值 10），真实会话中 glob/dir_list 的单行大 JSON 即此漏洞。
    let tok = super::tokenizer::default_tokenizer();
    if lines.len() <= threshold && tok.count(&full_text) <= MESSAGE_TOKEN_CAP {
        return Ok(None);
    }

    // 存档路径
    let archive_path = session_dir.join(archive_rel_path);

    // 确保存档父目录存在
    if let Some(parent) = archive_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| PluginError::InternalError(format!("创建消息存档目录失败: {e}")))?;
    }

    // 写入完整内容
    tokio::fs::write(&archive_path, full_text.as_bytes())
        .await
        .map_err(|e| PluginError::InternalError(format!("写入消息存档失败: {e}")))?;

    // 头尾保留（与 L0 split_head_tail 同策略）：首行常含结论/路径/计划骨架，
    // 只保尾部会把头部挤出模型视野（虽在存档，但模型不会主动取回）。
    // 头 = 1/4 阈值（至少 1 行），尾 = 其余；行数未超阈值（token 触发）时
    // 头 1 行 + 剩余全部行，随后走字符截断兜底。
    let head_keep = (threshold / 4).max(1);
    let tail_keep = threshold.saturating_sub(head_keep);
    let (head_lines, tail_lines) = if lines.len() > threshold {
        (&lines[..head_keep], &lines[lines.len() - tail_keep..])
    } else {
        (&lines[..1], &lines[1..])
    };
    let mut kept_lines: Vec<&str> = head_lines.to_vec();
    kept_lines.extend_from_slice(tail_lines);
    let kept_text = kept_lines.join("\n");

    // 单行超长（行数 ≤ threshold 但 token 超预算）时，保留行本身也超预算：
    // 按字符截断行首（与 split_head_tail 的单行退化策略一致）
    let kept_text = if tok.count(&kept_text) > MESSAGE_TOKEN_CAP {
        let char_cap = MESSAGE_TOKEN_CAP.saturating_mul(2);
        let truncated: String = kept_text.chars().take(char_cap).collect();
        format!("{truncated}…")
    } else {
        kept_text
    };

    // 构造压缩后内容：头部以注释形式注明完整路径，尾部为保留内容。
    // 单行 token 触发时行数描述与保留行数一致（都是 1 行），避免误导取回方。
    // 取回指引（P1-2 三层统一协议）：与 L0/L3 占位符共用同一格式 ——
    // 「已存档至: <路径> + 统一取回入口 local/file_read + 分段参数 offset/limit」。
    // 指引文案唯一来源：paths::RETRIEVAL_HINT（L0 tool_result_guard 同款）。
    let compressed_text = format!(
        "{COMPRESS_PREFIX} 完整内容已存档至: {archive_display_path} (共 {total_lines} 行){}, 以下保留开头 {head_count} 行与结尾 {tail_count} 行内容\n\
        ---\n\
        {kept_text}",
        super::paths::RETRIEVAL_HINT,
        total_lines = lines.len(),
        head_count = head_lines.len(),
        tail_count = tail_lines.len()
    );

    let mut compressed = msg.clone();
    compressed.content = Some(MessageContent::Text(compressed_text));

    // 将存档路径存入元数据，便于后续自动恢复
    let mut meta = msg.meta.clone().unwrap_or_else(|| serde_json::json!({}));
    meta["archive_path"] = serde_json::Value::String(archive_rel_path.to_string());
    compressed.meta = Some(meta);

    Ok(Some(compressed))
}

/// 恢复被压缩的消息。
///
/// 如果消息元数据中包含 `archive_path`，则从对应的存档文件读取完整内容并还原。
pub async fn decompress_message(
    session_dir: &Path,
    msg: &ChatMessage,
) -> Result<ChatMessage, PluginError> {
    let archive_path = msg
        .meta
        .as_ref()
        .and_then(|m| m.get("archive_path"))
        .and_then(|v| v.as_str());

    if let Some(rel_path) = archive_path {
        let full_path = session_dir.join(rel_path);
        if full_path.exists() {
            let content = tokio::fs::read_to_string(&full_path)
                .await
                .map_err(|e| PluginError::InternalError(format!("读取消息存档失败: {e}")))?;

            let mut restored = msg.clone();
            restored.content = Some(MessageContent::Text(content));

            // 还原后清理元数据中的存档路径标识
            if let Some(meta) = restored.meta.as_mut() {
                if let Some(obj) = meta.as_object_mut() {
                    obj.remove("archive_path");
                }
            }
            return Ok(restored);
        }
    }

    Ok(msg.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::session::chat_message::{MessageRole, MessageType};

    fn text_msg(s: &str) -> ChatMessage {
        ChatMessage {
            content: Some(MessageContent::Text(s.to_string())),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            ..Default::default()
        }
    }

    /// 单行超长内容：行数阈值判定失效，应按 token 预算兜底压缩并存档
    /// （真实会话实证：12k token 的单行 glob 结果因"只有 1 行"绕过压缩）
    #[tokio::test]
    async fn single_long_line_message_is_token_capped() {
        let dir = std::env::temp_dir().join(format!("symbio_msg_archive_t1_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let huge = format!("{{\"entries\":[\"{}\"]}}", "x".repeat(20_000));
        let msg = text_msg(&huge);

        let compressed = compress_message(&dir, &msg, 10, "messages/m1.txt", "messages/m1.txt")
            .await
            .unwrap()
            .expect("单行超长应触发压缩");

        let text = compressed.content.as_ref().map(|c| c.to_text()).unwrap_or_default();
        assert!(text.starts_with(COMPRESS_PREFIX), "压缩头应存在: {}", &text[..120.min(text.len())]);
        assert!(
            text.chars().count() < huge.chars().count() / 2,
            "压缩后应显著短于原文: {} vs {}",
            text.chars().count(),
            huge.chars().count()
        );
        // 存档含全文，meta 记录路径
        let archived = std::fs::read_to_string(dir.join("messages/m1.txt")).unwrap();
        assert_eq!(archived, huge);
        assert!(compressed.meta.as_ref().unwrap().get("archive_path").is_some());
        // 存档可完整还原
        let restored = decompress_message(&dir, &compressed).await.unwrap();
        assert_eq!(restored.content.as_ref().map(|c| c.to_text()).unwrap_or_default(), huge);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 短消息不压缩；多行超阈值消息行为不变（回归保护）
    #[tokio::test]
    async fn normal_messages_unaffected() {
        let dir = std::env::temp_dir().join(format!("symbio_msg_archive_t2_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        // 短消息：不压缩
        let short = compress_message(&dir, &text_msg("hello"), 10, "messages/m2.txt", "m2")
            .await
            .unwrap();
        assert!(short.is_none(), "短消息不应压缩");

        // 多行超阈值：仍按行压缩，头尾保留（P1-2 三层统一取回协议）
        let multi = text_msg(&(0..20).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n"));
        let compressed = compress_message(&dir, &multi, 10, "messages/m3.txt", "m3")
            .await
            .unwrap()
            .expect("多行超阈值应压缩");
        let text = compressed.content.as_ref().map(|c| c.to_text()).unwrap_or_default();
        assert!(
            text.contains("保留开头 2 行与结尾 8 行内容"),
            "头尾保留格式: {text}"
        );
        assert!(text.contains("line0"), "头部应保留: {text}");
        assert!(text.contains("line19"), "尾部应保留: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ToolCall 参数节点永久豁免：参数骨架化会让模型误记自身行为
    #[tokio::test]
    async fn toolcall_args_never_compressed() {
        let dir = std::env::temp_dir().join(format!("symbio_msg_archive_t3_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let args = (0..20).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        let mut msg = text_msg(&args);
        msg.msg_type = Some(MessageType::ToolCall);
        let result = compress_message(&dir, &msg, 10, "messages/m4.txt", "m4")
            .await
            .unwrap();
        assert!(result.is_none(), "ToolCall 参数应永久豁免");
        assert!(!dir.join("messages/m4.txt").exists(), "豁免时不应写存档");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 单行 + token 未超预算：门控放行不压缩（阈值门控另一半的回归保护）
    #[tokio::test]
    async fn long_line_under_token_cap_not_compressed() {
        let dir = std::env::temp_dir().join(format!("symbio_msg_archive_t4_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let one_line = format!("{{\"data\":\"{}\"}}", "y".repeat(800));
        let result = compress_message(&dir, &text_msg(&one_line), 10, "messages/m5.txt", "m5")
            .await
            .unwrap();
        assert!(result.is_none(), "token 未超预算的单行不应压缩");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
