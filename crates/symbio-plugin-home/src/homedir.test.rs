//! `plugins/home/homedir.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use std::sync::Mutex as StdMutex;

// 测试串行化（修改全局状态需要互斥）
static TEST_LOCK: StdMutex<()> = StdMutex::new(());

/// 取串行锁。**读全局状态的测试也必须取**——不只是写它的那些。
///
/// 踩过的坑：`test_default_homedir_is_user_home_dot_symbio` 起初没取锁，
/// 理由是"它只读、不改"。但同文件的其它测试会 `set(SYMBIO_HOMEDIR)`，于是
/// 它会在别人持有期间读到被改过的值而失败——**单独跑必过、全量跑随机红**。
/// 这种 flake 最坏的地方在于它看起来像"本次改动引入的"，会让人去查错误的
/// 方向（本次就是这么发现的：一次全量 720/1，重跑三遍全绿）。
fn lock_test() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn test_default_homedir_is_user_home_dot_symbio() {
    let _g = lock_test();
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
fn test_env_var_overrides_bootstrap() {
    // 契约：SYMBIO_HOMEDIR 是最高优先级，必须压过 bootstrap 文件。
    // 若 bootstrap 压过环境变量，显式指定的隔离系统目录（CI/E2E）
    // 会被静默覆盖，本测试防止该回归。
    let _g = lock_test();
    let original = std::env::var("SYMBIO_HOMEDIR").ok();
    let custom = std::env::temp_dir().join("symbio_env_priority_test");
    std::env::set_var("SYMBIO_HOMEDIR", &custom);
    // 无论本机 bootstrap 内容为何，环境变量必须胜出
    assert_eq!(initial_homedir(), custom);
    // 恢复环境变量，避免污染其他测试
    match original {
        Some(v) => std::env::set_var("SYMBIO_HOMEDIR", v),
        None => std::env::remove_var("SYMBIO_HOMEDIR"),
    }
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

    // 相对路径 → 相对 home 解析（兼容存量 bootstrap 中的相对路径写法）
    assert_eq!(normalize_homedir(".symbio").unwrap(), home.join(".symbio"));
    assert_eq!(normalize_homedir("foo/bar").unwrap(), home.join("foo/bar"));
}

#[test]
fn test_bootstrap_path_display_is_nonempty() {
    let s = HomedirRegistry::bootstrap_path_display();
    assert!(!s.is_empty());
}
