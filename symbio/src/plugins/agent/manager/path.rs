//! Agent 插件路径解析工具
//!
//! 集中所有"workdir → 物理路径"的展开逻辑，避免散落 `shellexpand::tilde()` 带来的：
//! - 路径未规范化（`..` 可逃逸`）
//! - 不同 OS 路径分隔符（`/` vs `\`）导致缓存击穿
//! - 难以做安全策略拦截
//!
//! 设计原则：
//! 1. **统一入口**：所有路径都通过 `resolve_workspace_dir` / `safe_join_under_home` 进入
//! 2. **轻量校验**：只做"是否存在 `..`"的字符串检查，避免引入 `path_clean` 等重型依赖
//! 3. **绝对化**：返回的路径全部是绝对路径，方便缓存 key
//!
//! ## 系统目录 (homedir)
//!
//! 全局 Agent 目录来自 [`crate::symbio_core::HomedirRegistry::get()`]，
//! 即当前系统目录的 `<homedir>/plugins/agent/`。

use crate::symbio_core::HomedirRegistry;
use std::path::{Component, Path, PathBuf};

/// 将 `~` 展开并构造工作区下的 `.symbio/agents` 目录
///
/// 返回 `None` 的条件：
/// - `workdir` 为 `None`（无工作区上下文）
/// - workdir 解析后不是绝对路径
/// - workdir 包含 `..` 路径分量（防止逃逸到工作区之外）
///
/// 返回的路径示例：
/// - `Some("/home/alice/myproj/.symbio/agents")`
pub fn resolve_workspace_dir(workdir: Option<&str>) -> Option<PathBuf> {
    let wd = workdir?;

    // 1. 检查 `..` 分量（防止路径逃逸）
    if has_parent_component(wd) {
        return None;
    }

    // 2. 展开 `~`
    let expanded = if wd == "~" || wd.starts_with("~/") {
        shellexpand::tilde(wd).to_string()
    } else {
        wd.to_string()
    };

    // 3. 拼装路径
    let p = PathBuf::from(expanded).join(".symbio").join("agents");

    // 4. 验证是绝对路径
    if !p.is_absolute() {
        return None;
    }

    Some(p)
}

/// 全局 Agent 目录：当前系统目录下的 `plugins/agent`
///
/// 与 `resolve_workspace_dir` 区别：此处是**全局共享**目录，路径是
/// `<homedir>/plugins/agent`（与其它插件的目录结构一致，
/// 便于通过遍历 `<homedir>/plugins/` 发现所有插件）。
///
/// homedir 默认 `~/.symbio`，可通过 [`HomedirRegistry`] 切换。
pub fn resolve_global_agents_dir() -> PathBuf {
    HomedirRegistry::get().join("plugins").join("agent")
}

/// 安全地拼接子路径到基础目录
///
/// - 禁止拼接后的路径逃逸出 `base`（通过 `..` 或绝对路径）
/// - 禁止空路径或仅 `..` 的输入
pub fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() {
        return None;
    }
    if has_parent_component(rel) {
        return None;
    }

    let p = base.join(rel);

    // 规范化后必须仍以 base 开头
    let normalized = normalize_path(&p);
    let base_normalized = normalize_path(base);
    if !normalized.starts_with(&base_normalized) {
        return None;
    }
    Some(normalized)
}

/// 简易路径规范化（处理 `.` 和 `..`）
///
/// 纯字符串实现，不依赖 `path_clean` crate；逻辑：
/// - 拆分路径分量
/// - 跳过 `.`
/// - 遇到 `..` 则弹出上一分量（若 base 不可越界）
fn normalize_path(p: &Path) -> PathBuf {
    let mut stack: Vec<PathBuf> = Vec::new();
    let mut absolute = false;

    for comp in p.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => {
                absolute = true;
                stack.clear();
                stack.push(comp.as_os_str().into());
            },
            Component::CurDir => {}, // skip "."
            Component::ParentDir => {
                // 只在有"非 prefix/root"分量可弹时弹
                if stack.len() > if absolute { 1 } else { 0 } {
                    stack.pop();
                }
            },
            Component::Normal(c) => stack.push(PathBuf::from(c)),
        }
    }

    let mut out = PathBuf::new();
    for s in &stack {
        out.push(s);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// 校验并规范化工作区根路径（不 join `.symbio/agents`）
///
/// 与 `resolve_workspace_dir` 的区别：
/// - `resolve_workspace_dir`：用于"agent 数据存储"——返回 `{workdir}/.symbio/agents`
/// - `validate_workspace_root`：用于"运行时工作目录"——只校验合法性，返回 workdir 本身
///
/// `None` 输入：透传 `None`（调用方需自行 fallback 到上游 ctx 的 workdir）
/// `Some("")`：返回 `None`（空字符串视为未指定）
/// `Some(..)`：
///   1. 拒绝包含 `..` 分量的路径（防逃逸）
///   2. 展开 `~` 前缀
///   3. 必须是绝对路径
///   4. 全部通过则返回规范化后的字符串
///
/// 返回 `None` 表示输入不合法（调用方应返回 400 错误）
pub fn validate_workspace_root(workdir: Option<&str>) -> Option<String> {
    let wd = workdir?;
    if wd.is_empty() {
        return None;
    }
    if has_parent_component(wd) {
        return None;
    }
    let expanded = if wd == "~" || wd.starts_with("~/") {
        shellexpand::tilde(wd).to_string()
    } else {
        wd.to_string()
    };
    if !Path::new(&expanded).is_absolute() {
        return None;
    }
    Some(expanded)
}

/// 生成缓存友好的规范化路径 key
///
/// 同一物理路径在不同 OS 上可能以不同字符串表示（`/` vs `\`）。
/// 此函数通过 `safe_join` 规范化后统一格式，避免缓存击穿。
///
/// 示例：
/// - `C:\Users\alice\.symbio\agents\my_agent` → `C:\Users\alice\.symbio\agents\my_agent\.`
/// - `/home/alice/.symbio/agents/my_agent` → `/home/alice/.symbio/agents/my_agent/.`
pub fn normalize_cache_key(path: &Path) -> String {
    // 使用 safe_join(".") 来触发路径规范化
    let normalized = safe_join(path, ".").unwrap_or_else(|| path.to_path_buf());
    normalized.to_string_lossy().to_string()
}

/// 检查路径字符串中是否包含 `..` 分量
fn has_parent_component(s: &str) -> bool {
    // 用 Path 的 components 准确判断（兼容 `/` 和 `\`）
    Path::new(s)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;
