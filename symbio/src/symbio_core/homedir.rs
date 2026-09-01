//! HomedirRegistry - 全局"系统目录"(homedir) 注册表
//!
//! ## 背景
//!
//! 历史上所有插件都硬编码 `dirs::home_dir().join(".symbio")` 作为系统目录，
//! 这导致用户无法在不改源码的情况下切换系统目录。
//!
//! 本模块提供：
//! 1. **运行时配置** 的系统目录 homedir（默认 `~/.symbio`，可由前端切换）
//! 2. **进程内全局单例** `HomedirRegistry::get()`，所有需要"系统根目录"的代码统一走这里
//! 3. **bootstrap 持久化**：把"上次使用的 homedir"写到 `~/.symbio_bootstrap`，
//!    下次启动时自动恢复。这样 bootstrap 文件本身在固定位置（用户主目录），
//!    不依赖 homedir 本身。
//!
//! ## 设计原则
//!
//! - **零侵入**：调用方只需把 `dirs::home_dir().join(".symbio")` 改成
//!   `HomedirRegistry::get()` 即可获得 homedir 切换能力。
//! - **向后兼容**：默认 homedir 仍为 `~/.symbio`，存量用户行为零变化。
//! - **可测试**：`set()` / `reset_to_default()` 暴露给测试，验证 set/get 一致性。
//! - **bootstrap 容错**：bootstrap 文件不存在 / 解析失败 / 路径不存在，
//!   都回退到默认 `~/.symbio` 并打 warn 日志，不阻断应用启动。
//!
//! ## 限制
//!
//! - 由于 plugin 的 `build()` 是同步函数，无法在 `build` 内部从 InvokeRequest 拿到
//!   homedir 上下文（构造时还没建好 InvokeRequest）。所以 homedir 必须是**进程级
//!   全局变量**而不是请求级 context。这与 `dirs::home_dir()` 静态全局的设计一致。
//! - 切换 homedir **不会**自动迁移数据（避免误删），由用户在 UI 显式选择。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tracing::{info, warn};

/// 默认 homedir（向后兼容）
///
/// 使用 `~/.symbio` 形式，调用 [`expand_tilde_path`] 时会展开为
/// `<user_home>/.symbio`（**绝对路径**）。
pub const DEFAULT_HOMEDIR: &str = "~/.symbio";

/// bootstrap 文件名（位于用户主目录下，不受 homedir 切换影响）
const BOOTSTRAP_FILENAME: &str = ".symbio_bootstrap";

/// bootstrap 文件内容
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Bootstrap {
    /// 用户最近选择的 homedir
    homedir: String,
}

/// 全局 homedir 注册表
///
/// 内部使用 `OnceLock` + `Mutex` 组合：
/// - `OnceLock` 保证初始化一次（init_from_bootstrap）
/// - `Mutex` 允许 set() 后修改
struct Inner {
    current: PathBuf,
}

/// 全局状态
static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();

/// 初始化全局状态（进程内只调用一次）
fn inner() -> &'static Mutex<Inner> {
    INNER.get_or_init(|| {
        let p = load_from_bootstrap_or_default();
        Mutex::new(Inner { current: p })
    })
}

/// 计算默认 homedir
///
/// 优先级：
/// 1. `SYMBIO_HOMEDIR` 环境变量（便于开发/CI/容器）
/// 2. `<user_home>/.symbio`
fn default_homedir() -> PathBuf {
    if let Ok(env_p) = std::env::var("SYMBIO_HOMEDIR") {
        if !env_p.trim().is_empty() {
            return expand_tilde_path(Path::new(&env_p));
        }
    }
    expand_tilde_path(Path::new(DEFAULT_HOMEDIR))
}

/// 展开 `~` 前缀到用户主目录
pub fn expand_tilde_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(stripped) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(s.as_ref())
}

/// bootstrap 文件路径：固定位于用户主目录下
fn bootstrap_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(BOOTSTRAP_FILENAME))
}

/// 归一化 homedir 路径为绝对路径
///
/// 行为：
/// - 空路径 → 返回 `None`（调用方应回退默认）
/// - `~` / `~/xxx` → 展开为 `<user_home>[/xxx]`
/// - 已是绝对路径 → 原样返回
/// - 相对路径（如存量 bootstrap 中存的 `.symbio`）→ 解析为 `<user_home>/<relative>`
///   （向后兼容早期版本的 bootstrap 内容）
fn normalize_homedir(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = expand_tilde_path(Path::new(trimmed));
    if expanded.is_absolute() {
        return Some(expanded);
    }
    // 相对路径 → 视为相对用户主目录
    if let Some(home) = dirs::home_dir() {
        return Some(home.join(&expanded));
    }
    Some(expanded)
}

/// 从 bootstrap 文件加载 homedir；失败时回退到默认
fn load_from_bootstrap_or_default() -> PathBuf {
    let Some(path) = bootstrap_path() else {
        return default_homedir();
    };
    if !path.exists() {
        return default_homedir();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_yaml_ng::from_str::<Bootstrap>(&content) {
            Ok(b) => match normalize_homedir(&b.homedir) {
                Some(p) => {
                    info!(
                        bootstrap = %path.display(),
                        homedir = %p.display(),
                        "HomedirRegistry: 从 bootstrap 恢复 homedir"
                    );
                    p
                }
                None => {
                    warn!(path = %path.display(), "bootstrap.homedir 为空，使用默认 homedir");
                    default_homedir()
                }
            },
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "bootstrap 文件解析失败，使用默认 homedir"
                );
                default_homedir()
            }
        },
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "bootstrap 文件读取失败，使用默认 homedir"
            );
            default_homedir()
        }
    }
}

/// 全局 homedir 注册表
pub struct HomedirRegistry;

impl HomedirRegistry {
    /// 获取当前 homedir（一定不会 panic，且不需要可变借用）
    ///
    /// 首次调用会触发 `load_from_bootstrap_or_default()`，后续调用直接返回缓存值。
    pub fn get() -> PathBuf {
        inner().lock().unwrap().current.clone()
    }

    /// 切换 homedir 并持久化到 bootstrap 文件
    ///
    /// - 立即更新内存中的值
    /// - 同步写入 bootstrap 文件（保证下次启动恢复）
    /// - 不创建/迁移 homedir 目录本身（由调用方按需处理）
    ///
    /// 返回旧 homedir 便于调用方对比。
    pub fn set(new_homedir: PathBuf) -> std::io::Result<PathBuf> {
        let new_homedir = match normalize_homedir(&new_homedir.to_string_lossy()) {
            Some(p) => p,
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "homedir 不能为空",
                ));
            }
        };

        // 1. 写入 bootstrap（先持久化，再改内存，保证原子性）
        let path = bootstrap_path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "无法获取用户主目录")
        })?;
        let content = serde_yaml_ng::to_string(&Bootstrap {
            homedir: new_homedir.to_string_lossy().to_string(),
        })
        .map_err(|e| std::io::Error::other(format!("序列化 bootstrap 失败: {e}")))?;
        std::fs::write(&path, content)?;

        // 2. 更新内存
        let mut guard = inner().lock().unwrap();
        let old = guard.current.clone();
        guard.current = new_homedir.clone();

        info!(
            old = %old.display(),
            new = %new_homedir.display(),
            bootstrap = %path.display(),
            "HomedirRegistry: homedir 已切换并持久化"
        );
        Ok(old)
    }

    /// 重置为默认 homedir（测试 / "恢复默认" 按钮使用）
    pub fn reset_to_default() -> PathBuf {
        let new_default = default_homedir();
        // 不删 bootstrap 文件，直接覆盖
        let _ = Self::set(new_default.clone());
        new_default
    }

    /// 返回 bootstrap 文件路径（供前端"在哪里存"提示）
    pub fn bootstrap_path_display() -> String {
        bootstrap_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "(无法获取用户主目录)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // 测试串行化（修改全局状态需要互斥）
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn lock_test() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn test_default_homedir_is_user_home_dot_symbio() {
        // SYMBIO_HOMEDIR 未设置时，应返回 `<user_home>/.symbio`（**绝对路径**）
        // 不能直接断言绝对路径（CI 上 home 不同），但需断言为绝对路径且以 ".symbio" 结尾
        let p = default_homedir();
        assert!(
            p.is_absolute(),
            "default_homedir 必须是绝对路径, got: {}",
            p.display()
        );
        let s = p.to_string_lossy();
        let trimmed = s.trim_end_matches(['/', '\\']);
        assert!(
            trimmed.ends_with(".symbio"),
            "default_homedir 应以 .symbio 结尾, got: {s}"
        );
    }

    #[test]
    fn test_set_and_get_roundtrip() {
        let _g = lock_test();
        let original = HomedirRegistry::get();

        let custom = std::env::temp_dir().join("symbio_homedir_test_unique_xyz");
        let _ = std::fs::remove_dir_all(&custom);
        let _ = std::fs::create_dir_all(&custom);

        let old = HomedirRegistry::set(custom.clone()).expect("set 成功");
        assert_eq!(old, original);
        assert_eq!(HomedirRegistry::get(), custom);

        // 恢复
        let _ = HomedirRegistry::set(original.clone());
        assert_eq!(HomedirRegistry::get(), original);

        let _ = std::fs::remove_dir_all(&custom);
    }

    #[test]
    fn test_set_empty_returns_error() {
        let _g = lock_test();
        let r = HomedirRegistry::set(PathBuf::new());
        assert!(r.is_err(), "空路径应返回错误");
    }

    #[test]
    fn test_normalize_homedir() {
        // 空 → None
        assert!(normalize_homedir("").is_none());
        assert!(normalize_homedir("   ").is_none());

        // ~ / ~/xxx → 展开
        let home = dirs::home_dir().unwrap();
        assert_eq!(normalize_homedir("~").unwrap(), home);
        assert_eq!(normalize_homedir("~/foo").unwrap(), home.join("foo"));

        // 绝对路径 → 原样
        let abs = std::env::temp_dir().join("abs_path");
        assert_eq!(normalize_homedir(&abs.to_string_lossy()).unwrap(), abs);

        // 相对路径 → 相对 home 解析（向后兼容存量 bootstrap）
        assert_eq!(normalize_homedir(".symbio").unwrap(), home.join(".symbio"));
        assert_eq!(normalize_homedir("foo/bar").unwrap(), home.join("foo/bar"));
    }

    #[test]
    fn test_expand_tilde() {
        let p = expand_tilde_path(Path::new("~"));
        assert_eq!(p, dirs::home_dir().unwrap());

        let p = expand_tilde_path(Path::new("~/foo/bar"));
        let expected = dirs::home_dir().unwrap().join("foo/bar");
        assert_eq!(p, expected);

        // 绝对路径不变
        let abs = std::env::temp_dir().join("test");
        let p = expand_tilde_path(&abs);
        assert_eq!(p, abs);

        // 普通相对路径直接返回
        let p = expand_tilde_path(Path::new("relative/path"));
        assert_eq!(p, PathBuf::from("relative/path"));
    }

    #[test]
    fn test_bootstrap_path_display_is_nonempty() {
        let s = HomedirRegistry::bootstrap_path_display();
        assert!(!s.is_empty());
    }
}
