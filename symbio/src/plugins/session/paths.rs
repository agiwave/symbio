//! 会话存储路径工具 —— 会话 ID 到文件系统目录的映射唯一权威实现。
//!
//! Session 插件内所有"落盘到会话目录"的组件（存储后端、L0 工具结果守卫、
//! L1 消息存档、L3 transcript 转存）都必须经由本模块拼路径，禁止各自
//! 重复实现 `safe_id` 或手工重建 `<homedir>/plugins/session` 前缀：
//!
//! - [`safe_id`]：session_id → 安全目录名（历史教训：同一替换逻辑曾在
//!   store/file.rs、tool_result_guard.rs、chat_loop.rs、chat_session.rs
//!   各写一份，行为漂移风险高，故收敛于此）。
//! - [`session_dir`] / [`session_subdir`]：基于
//!   [`SessionPlugin::session_storage_dir`]（其注释声明为存储根目录的
//!   唯一权威位置）派生会话目录与会话内子目录。

/// 会话内固定子目录名：L0 工具结果全文存档。
pub const TOOL_ARCHIVES_SUBDIR: &str = "tool_archives";

/// 会话内固定子目录名：L3 压缩前完整历史 transcript 转存。
pub const TRANSCRIPTS_SUBDIR: &str = "transcripts";

/// 会话内固定子目录名：L1 单条消息内容存档。
pub const MESSAGES_SUBDIR: &str = "messages";

/// 三层压缩占位符共用的统一取回指引（P1-2 协议）。
///
/// L0（工具结果守卫）、L1（消息存档）、L3（transcript 转存）的占位符
/// 都必须附带同一格式的取回说明，保证模型在任意层级遇到存档占位符时
/// 都能用同一入口（`local/file_read` + offset/limit 分段）取回全文。
pub const RETRIEVAL_HINT: &str = "（取回：local/file_read 该路径，按 offset/limit 分段读取）";

/// 将 session_id 转换为安全的目录名。
///
/// session_id 会直接成为文件系统目录名，必须替换路径分隔符（`/`、`\`）
/// 与 Windows 盘符冒号（`:`），防止路径穿越与非法目录名。
/// 空串/空白原样返回（不做 trim），与历史实现行为一致。
pub(crate) fn safe_id(session_id: &str) -> String {
    session_id.replace(['/', '\\', ':'], "_")
}

/// 会话目录：`<homedir>/plugins/session/<safe_id>/`
pub(crate) fn session_dir(session_id: &str) -> std::path::PathBuf {
    super::plugin::SessionPlugin::session_storage_dir().join(safe_id(session_id))
}

/// 会话内子目录：`<homedir>/plugins/session/<safe_id>/<subdir>/`
///
/// 用于 tool_archives / transcripts / messages 等固定子目录
///（常量见本模块 [`TOOL_ARCHIVES_SUBDIR`] / [`TRANSCRIPTS_SUBDIR`] / [`MESSAGES_SUBDIR`]）。
pub(crate) fn session_subdir(session_id: &str, subdir: &str) -> std::path::PathBuf {
    session_dir(session_id).join(subdir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_id_replaces_path_separators_and_colon() {
        assert_eq!(safe_id("a/b"), "a_b");
        assert_eq!(safe_id("a\\b"), "a_b");
        assert_eq!(safe_id("c:tmp"), "c_tmp");
        assert_eq!(safe_id("a/b\\c:d"), "a_b_c_d");
    }

    #[test]
    fn safe_id_keeps_normal_ids_untouched() {
        assert_eq!(safe_id("plain-id_123"), "plain-id_123");
        assert_eq!(safe_id(""), "");
    }

    #[test]
    fn retrieval_hint_mentions_unified_entry() {
        assert!(!RETRIEVAL_HINT.is_empty());
        assert!(RETRIEVAL_HINT.contains("local/file_read"));
        assert!(RETRIEVAL_HINT.contains("offset/limit"));
    }

    #[test]
    fn session_subdir_appends_under_safe_session_dir() {
        let dir = session_subdir("a/b", TOOL_ARCHIVES_SUBDIR);
        let parts: Vec<_> = dir.components().collect();
        // 末两段应为 safe_id 段与子目录段
        assert_eq!(parts.len() >= 2, true);
        let last_two: Vec<String> = parts[parts.len() - 2..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            last_two,
            vec!["a_b".to_string(), "tool_archives".to_string()]
        );
    }
}
