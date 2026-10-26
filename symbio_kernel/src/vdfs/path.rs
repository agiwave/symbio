//! 路径判定（地址契约的唯一实现，纯词汇面）。
//!
//! 下面两个函数是**地址规则**的实现：任何按路径段比较、或需要收敛坐标系的场合
//! 都调用它们，不得各写一份。历史教训是「按字符串前缀代替按路径段比较」：
//! Windows 的 `src\..\..\..\Windows` 曾绕过守卫，裸 `starts_with("/etc")` 曾误伤
//! `/etcfoo`。

/// 路径中是否含 `..` 段——**两种分隔符都算**。
///
/// 只查 `/` 会让 Windows 的 `src\..\..\..\Windows` 绕过守卫；只查带分隔符的
/// `../` / `..\` 前缀会放过 `a/..`（`..` 收尾）。因此按**段**判定，与分隔符无关。
pub fn vdfs_has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|seg| seg == "..")
}

/// `path` 是否落在 `prefix` 之内——相等，或紧随一个分隔符。
///
/// 前缀必须按**路径段**比较：裸 `starts_with("/etc")` 会把 `/etcfoo` 误伤。
pub fn vdfs_path_within(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches(['/', '\\']);
    if prefix.is_empty() {
        return false;
    }
    match path.strip_prefix(prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'),
        None => false,
    }
}
