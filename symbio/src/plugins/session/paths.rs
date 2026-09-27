//! Session 插件的路径派生 —— 会话寻址的**双向映射入口**。
//!
//! Session 插件内所有"落盘到会话目录"的组件（存储后端、L0 工具结果守卫、
//! L3 transcript 转存）都必须经由本模块拼路径，禁止各自
//! 重复实现 `safe_id` 或手工重建 `<本插件目录>` 前缀：
//!
//! - [`safe_id`]：session_id → 安全目录名。**规则本身不在本模块**——它是
//!   宿主层 [`crate::providers::vdfs_safe_segment`]（所有 VDFS
//!   资源条目共用的那一版，含 `.` / `..` 与控制字符防护），本模块只是会话侧的
//!   入口。历史教训：同一替换逻辑曾在 store/file.rs、tools/tool_result_guard.rs、
//!   chat_loop.rs、chat_session.rs 各写一份，行为漂移风险高；收敛到插件内唯一
//!   之后仍与宿主层并存的第二版，故再上一台阶。
//! - [`session_dir`] / [`session_subdir`]：会话目录与会话内子目录。
//!   **根由调用方给**（装配态即本插件自己的目录，由父插件经 `PLUGIN_DIR` 告知）——
//!   本模块只做「id → 目录名」这一段，不认识任何全局布局。
//! - [`session_id_from_new_path`]：**反向**的那一段——VDFS 写地址 → 会话 id
//!   （具名新建时「地址末段即身份」，写挂载根这种无名目标交 provider 生成）。
//!   它与上面的「id → 目录名」是同一套寻址规则的两个方向，故同处一模块；
//!   原先住在 `plugin/nodes.rs` 的路径模型里（见该模块头），归位理由是
//!   消费方在**领域层**（会话的写语义要按地址定 id），而领域层不引用汇编层。

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
    crate::providers::vdfs_safe_segment(session_id)
}

/// 会话目录：`<会话存储根>/<safe_id>/`
///
/// `root` = 会话存储根（本插件自己的目录）。
pub(crate) fn session_dir(root: &std::path::Path, session_id: &str) -> std::path::PathBuf {
    root.join(safe_id(session_id))
}

/// 会话内子目录：`<会话存储根>/<safe_id>/<subdir>/`
///
/// 用于 tool_archives / transcripts 等固定子目录
///（常量见本模块 [`TOOL_ARCHIVES_SUBDIR`] / [`TRANSCRIPTS_SUBDIR`]）。
pub(crate) fn session_subdir(
    root: &std::path::Path,
    session_id: &str,
    subdir: &str,
) -> std::path::PathBuf {
    session_dir(root, session_id).join(subdir)
}

/// 具名新建时，**地址末段即会话 id**；写在挂载根（无名目标）返回 `None`。
///
/// 「**有名字**时 id 来自地址（使用方给），**没名字**时 id 由 provider 生成」是
/// VDFS 的**通用**规则，两处规范同义：`providers/vdfs_service/entry.rs::id_of`
/// 的注释，以及 [`crate::symbio_core::vdfs::VdfsRequest::Write`] 的「两种目标形态」
/// 表（具名节点 + 不存在 ⇒ **就地创建**；只有目录自身才「名字由 provider 生成」）。
///
/// ## 为什么不剥 `.session` 后缀
///
/// 会话寻址里扩展名**从来不是**地址的一部分，也**从来不被剥除**：
/// `Session(id)` 直接把末段当 id 用（`parse_session_path` → `session_of`）。
/// 只在这里剥会造出「同一个 id 有两种写法、其中一种只在新建时成立」的怪状态，
/// 比不剥更糟。
pub(crate) fn session_id_from_new_path(path: &str) -> Option<String> {
    let base = path.rsplit('/').next().unwrap_or(path).trim();
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

#[cfg(test)]
#[path = "paths.test.rs"]
mod tests;
