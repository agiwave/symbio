//! 日志级别闸门的回归锚。
//!
//! 背景：插件日志宏在**没有 subscriber** 时退回 `eprintln!`，这条路径没有过滤器。
//! 若不设闸门，`plugin_debug!` 会无条件打到 stderr——启动期「正在构造子插件 …」
//! 每插件一行，用户每次启动都要看十几行纯机械噪声。

use super::*;

/// 级别名解析：四档 + 两个别名；无法识别 → `None`（调用方保持现值，不误伤）。
#[test]
fn parse_level_maps_names_and_rejects_junk() {
    assert_eq!(logger_parse_level("debug"), Some(LOG_LEVEL_DEBUG));
    assert_eq!(logger_parse_level("info"), Some(LOG_LEVEL_INFO));
    assert_eq!(logger_parse_level("warn"), Some(LOG_LEVEL_WARN));
    assert_eq!(logger_parse_level("error"), Some(LOG_LEVEL_ERROR));
    // `trace` 归 debug（本系统无更细档位）、`off` 归 error（等价于「几乎不输出」）
    assert_eq!(logger_parse_level("trace"), Some(LOG_LEVEL_DEBUG));
    assert_eq!(logger_parse_level("off"), Some(LOG_LEVEL_ERROR));
    // 大小写与首尾空白不敏感——环境变量是手写的
    assert_eq!(logger_parse_level("  DEBUG "), Some(LOG_LEVEL_DEBUG));
    assert_eq!(logger_parse_level("Warning"), Some(LOG_LEVEL_WARN));
    // 不认识的词不改变现状
    assert_eq!(logger_parse_level("verbose"), None);
    assert_eq!(logger_parse_level(""), None);
}

/// 闸门：默认 INFO 下 debug 静默；放开到 DEBUG 后放行；更严的级别不被放开影响。
///
/// 与全局状态打交道（`MIN_LEVEL` 是进程级原子量），故**首尾都复位**到默认 INFO，
/// 且这是本二进制内唯一触碰该状态的用例。
#[test]
fn gate_is_info_by_default_and_opens_for_debug() {
    // 装了订阅器时过闸由 `EnvFilter` 负责，本闸门恒放行——在这种二进制里无力断言。
    if logger_is_initialized() {
        return;
    }

    logger_set_min_level(LOG_LEVEL_INFO);
    assert!(!logger_level_enabled(LOG_LEVEL_DEBUG), "默认不得输出 debug");
    assert!(logger_level_enabled(LOG_LEVEL_INFO), "默认必须输出 info");
    assert!(logger_level_enabled(LOG_LEVEL_WARN));
    assert!(logger_level_enabled(LOG_LEVEL_ERROR));

    logger_set_min_level(LOG_LEVEL_DEBUG);
    assert!(logger_level_enabled(LOG_LEVEL_DEBUG), "放开后 debug 应可见");

    // 更严的级别（error）不受 debug 放开影响：error 仍可见，info 被挡
    logger_set_min_level(LOG_LEVEL_ERROR);
    assert!(!logger_level_enabled(LOG_LEVEL_INFO));
    assert!(!logger_level_enabled(LOG_LEVEL_DEBUG));
    assert!(logger_level_enabled(LOG_LEVEL_ERROR));

    // 复位：避免污染同二进制内的其它用例
    logger_set_min_level(LOG_LEVEL_INFO);
    assert_eq!(logger_min_level(), LOG_LEVEL_INFO);
}
