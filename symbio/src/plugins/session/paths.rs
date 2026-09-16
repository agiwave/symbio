//! Session 插件的路径派生 —— 会话 ID 到文件系统目录的映射入口。
//!
//! Session 插件内所有"落盘到会话目录"的组件（存储后端、L0 工具结果守卫、
//! L3 transcript 转存）都必须经由本模块拼路径，禁止各自
//! 重复实现 `safe_id` 或手工重建 `<本插件目录>` 前缀：
//!
//! - [`safe_id`]：session_id → 安全目录名。**规则本身不在本模块**——它是
//!   宿主层 [`crate::providers::vdfs_service::entry::safe_segment`]（所有 VDFS
//!   资源条目共用的那一版，含 `.` / `..` 与控制字符防护），本模块只是会话侧的
//!   入口。历史教训：同一替换逻辑曾在 store/file.rs、tool_result_guard.rs、
//!   chat_loop.rs、chat_session.rs 各写一份，行为漂移风险高；收敛到插件内唯一
//!   之后仍与宿主层并存的第二版，故再上一台阶。
//! - [`session_dir`] / [`session_subdir`]：基于
//!   [`SessionPlugin::session_storage_dir`]（同样委托宿主层 `category_dir`）
//!   派生会话目录与会话内子目录。

/// 会话内固定子目录名：L0 工具结果全文存档。
pub const TOOL_ARCHIVES_SUBDIR: &str = "tool_archives";

/// 会话内固定子目录名：L3 压缩前完整历史 transcript 转存。
pub const TRANSCRIPTS_SUBDIR: &str = "transcripts";

/// 压缩占位符共用的统一取回指引（P1-2 协议）。
///
/// L0（工具结果守卫）、L3（transcript 转存）的占位符都必须附带同一格式的
/// 取回说明，保证模型在任意层级遇到存档占位符时都能用同一入口
///（`vdfs_read` + offset/limit 分段）取回全文。
/// 请求视图层的淡化（内容节点/工具结果）不写存档，原文恒在会话存储中，
/// 无需取回指引。
pub const RETRIEVAL_HINT: &str = "（取回：vdfs_read 该路径，按 offset/limit 分段读取）";

/// 将 session_id 转换为安全的目录名。
///
/// session_id 会直接成为文件系统目录名，必须替换路径分隔符（`/`、`\`）
/// 与 Windows 盘符冒号（`:`），防止路径穿越与非法目录名。规则与 VDFS 资源
/// 条目共用一份（见模块头），因此同一 id 在会话目录与资源目录下的映射不会分叉。
pub(crate) fn safe_id(session_id: &str) -> String {
    crate::providers::vdfs_service::entry::safe_segment(session_id)
}

/// 会话目录：`<本插件目录>/<safe_id>/`
pub(crate) fn session_dir(session_id: &str) -> std::path::PathBuf {
    super::plugin::SessionPlugin::session_storage_dir().join(safe_id(session_id))
}

/// 会话内子目录：`<本插件目录>/<safe_id>/<subdir>/`
///
/// 用于 tool_archives / transcripts 等固定子目录
///（常量见本模块 [`TOOL_ARCHIVES_SUBDIR`] / [`TRANSCRIPTS_SUBDIR`]）。
pub(crate) fn session_subdir(session_id: &str, subdir: &str) -> std::path::PathBuf {
    session_dir(session_id).join(subdir)
}

#[cfg(test)]
#[path = "paths.test.rs"]
mod tests;
