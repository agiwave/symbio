//! 路径判定（地址契约的唯一实现，纯词汇面）。
//!
//! 下面三个函数是**地址规则**的实现：任何按路径段比较、或需要收敛坐标系的场合
//! 都调用它们，不得各写一份。历史教训是「按字符串前缀代替按路径段比较」：
//! Windows 的 `src\..\..\..\Windows` 曾绕过守卫，裸 `starts_with("/etc")` 曾误伤
//! `/etcfoo`。

/// 路径中是否含 `..` 段——**两种分隔符都算**。
///
/// 只查 `/` 会让 Windows 的 `src\..\..\..\Windows` 绕过守卫；只查带分隔符的
/// `../../` / `..\` 前缀会放过 `a/..`（`..` 收尾）。因此按**段**判定，与分隔符无关。
pub fn vdfs_has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|seg| seg.trim() == "..")
}

/// Windows 词法判定：路径段是否携带 NTFS 备用数据流名。
///
/// 只放过首段的标准盘符；`canonicalize` 会隐藏流名，因此调用方必须在 I/O 前判定。
pub fn vdfs_has_alternate_data_stream(path: &str) -> bool {
    let path = vdfs_normalize_path(path);
    path.split('/').enumerate().any(|(index, segment)| {
        let drive_prefix = index == 0
            && segment.len() == 2
            && segment.as_bytes()[0].is_ascii_alphabetic()
            && segment.as_bytes()[1] == b':';
        segment.contains(':') && !drive_prefix
    })
}

/// 地址的词法归一化：统一分隔符、逐段去除首尾空白、折叠重复分隔符与 `.`，保留绝对锚点和 `..`。
///
/// 这里**不解析** `..`：事实层必须让穿越意图继续可见，物理层再按自己的边界拒绝。
pub fn vdfs_normalize_path(path: &str) -> String {
    let mut value = path.trim().replace('\\', "/");
    if let Some(rest) = value.strip_prefix("//?/") {
        value = rest.to_string();
    } else if let Some(rest) = value.strip_prefix("//./") {
        value = rest.to_string();
    }
    if value.as_bytes().get(1) == Some(&b':') && value.as_bytes()[0].is_ascii_alphabetic() {
        value[..1].make_ascii_lowercase();
    }
    let anchor_len = value.chars().take_while(|c| *c == '/').count().min(2);
    let (anchor, body) = value.split_at(anchor_len);
    let mut segments: Vec<String> = body
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(str::to_string)
        .collect();
    if let Some(first) = segments.first_mut() {
        if first.len() == 2
            && first.as_bytes()[1] == b':'
            && first.as_bytes()[0].is_ascii_alphabetic()
        {
            first.make_ascii_lowercase();
        }
    }
    let body = segments.join("/");
    format!("{anchor}{body}")
}

/// 相对路径按调用方工作目录解析为事实坐标；绝对路径原样归一化。
///
/// Windows 的盘符相对路径（`C:foo`）不是绝对路径，也不能接到工作目录后冒充绝对路径，
/// 因而原样交给策略层拒绝。UNC 与 POSIX 的首部锚点均保留。
pub fn vdfs_resolve_path(path: &str, base: &str) -> String {
    let normalized = vdfs_normalize_path(path);
    let drive_relative = normalized.as_bytes().get(1) == Some(&b':')
        && normalized
            .as_bytes()
            .get(2)
            .is_none_or(|byte| *byte != b'/');
    if normalized.starts_with('/')
        || (normalized.as_bytes().get(1) == Some(&b':') && !drive_relative)
    {
        return normalized;
    }
    if drive_relative {
        return normalized;
    }
    let base = vdfs_normalize_path(base);
    if base.is_empty() {
        normalized
    } else {
        format!("{}/{}", base.trim_end_matches('/'), normalized)
    }
}

/// `path` 是否落在 `prefix` 之内——相等，或紧随一个分隔符。
///
/// 前缀必须按**路径段**比较：裸 `starts_with("/etc")` 会把 `/etcfoo` 误伤。
pub fn vdfs_path_within(path: &str, prefix: &str) -> bool {
    let path = vdfs_normalize_path(path);
    let prefix = vdfs_normalize_path(prefix);
    #[cfg(windows)]
    let (path, prefix) = (path.to_ascii_lowercase(), prefix.to_ascii_lowercase());
    if prefix.is_empty() {
        return false;
    }
    match path.strip_prefix(&prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
}
