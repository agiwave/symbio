//! `apps/desktop/src/main.rs` 的单元测试。
//!
//! 环境变量是进程全局状态，并行测试会互相踩踏，故凡是读写 env 的用例都在同一个
//! `#[test]` 里串行完成。

use super::*;

#[test]
fn cli_exe_name_matches_platform() {
    let expected = if cfg!(windows) {
        "symbio-cli.exe"
    } else {
        "symbio-cli"
    };
    assert_eq!(cli_exe_name(), expected);
}

#[test]
fn homedir_env_override_and_platform_default() {
    // 显式声明优先。
    std::env::set_var("SYMBIO_HOMEDIR", "test-desktop-homedir");
    assert_eq!(
        resolve_homedir().unwrap(),
        PathBuf::from("test-desktop-homedir")
    );

    // 空值视作未设置，回落到平台配置根。
    std::env::set_var("SYMBIO_HOMEDIR", "");
    let got = resolve_homedir().unwrap();
    std::env::remove_var("SYMBIO_HOMEDIR");
    assert!(
        got.ends_with("symbio"),
        "homedir 应落在用户配置目录下的 symbio: {}",
        got.display()
    );
}

#[test]
fn cli_binary_env_override_and_missing_path() {
    let dir = std::env::temp_dir();
    let fake = dir.join("symbio-cli-test");
    std::fs::write(&fake, b"fake").unwrap();

    std::env::set_var("SYMBIO_CLI", &fake);
    assert_eq!(resolve_cli_binary().unwrap(), fake);

    std::env::set_var("SYMBIO_CLI", "definitely-not-a-real-symbio-cli-binary");
    assert!(
        resolve_cli_binary().is_err(),
        "显式声明的 CLI 路径不存在时应报错，而不是静默回落到 PATH"
    );

    std::env::remove_var("SYMBIO_CLI");
    std::fs::remove_file(&fake).unwrap();
}
