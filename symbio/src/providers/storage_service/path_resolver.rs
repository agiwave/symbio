//! 路径解析工具
//!
//! - 将 ID 转换为安全的子目录名

/// 将 ID 转换为安全的子目录名
///
/// - `/` `\` `:` `*` `?` `"` `<` `>` `|` 替换为 `_`
/// - 去除前后空白
/// - 禁止 `.` 和 `..`
pub fn safe_id(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return "_empty_".to_string();
    }
    if trimmed == "." || trimmed == ".." {
        return format!("_{}_", trimmed);
    }
    // 替换不安全字符
    let mut s = trimmed.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    // 防止出现 NUL 等控制字符
    s = s
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect();
    s
}
