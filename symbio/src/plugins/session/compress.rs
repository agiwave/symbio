//! 会话消息内容压缩模块
//!
//! 压缩策略（针对单条消息）：
//! - 如果一条消息的内容行数超过阈值（默认 10 行），则：
//!   1. 将完整内容写入会话目录下的存档文件（messages/msg_{id}_{ts}.txt）
//!   2. 在消息内容中只保留最后 10 行
//!   3. 在保留内容头部追加一段系统注释，标明完整内容的相对路径
//! - 不足阈值的消息原样返回

use super::types::ChatMessage;
use crate::symbio_core::schemas::session::chat_message::MessageContent;
use crate::symbio_core::PluginError;
use std::path::Path;

/// 存档子目录名（位于会话目录内）
pub const MESSAGES_SUBDIR: &str = "messages";

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

    let lines: Vec<&str> = full_text.lines().collect();

    // 2. 检查行数阈值
    if lines.len() <= threshold {
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

    // 保留最后 threshold 行
    let kept_lines = &lines[lines.len() - threshold..];
    let kept_text = kept_lines.join("\n");

    // 构造压缩后内容：头部以注释形式注明完整路径，尾部为保留内容
    let compressed_text = format!(
        "{COMPRESS_PREFIX} 完整内容已存档至: {archive_display_path} (共 {total_lines} 行), 以下是最后 {threshold} 行内容\n\
        ---\n\
        {kept_text}",
        total_lines = lines.len()
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
