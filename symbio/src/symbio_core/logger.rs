//! 结构化日志系统
//!
//! 使用 tracing 提供统一的日志接口，支持级别过滤和字段附加。

use std::sync::OnceLock;

/// 全局日志订阅器状态
static LOGGER_INITIALIZED: OnceLock<()> = OnceLock::new();

/// 初始化日志系统，通过 RUST_LOG 环境变量配置
pub fn init_logger() {
    if LOGGER_INITIALIZED.set(()).is_err() {
        return; // 已初始化
    }

    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,symbio=debug")),
        )
        .init();
}

/// 检查是否已初始化
pub fn is_logger_initialized() -> bool {
    LOGGER_INITIALIZED.get().is_some()
}

/// 插件级别信息日志
#[macro_export]
macro_rules! plugin_info {
    ($plugin:expr, $($arg:tt)*) => {
        if $crate::symbio_core::is_logger_initialized() {
            tracing::info!(plugin = %$plugin, $($arg)*);
        } else {
            eprintln!("[{} INFO] {}", $plugin, format_args!($($arg)*));
        }
    };
}

/// 插件级别调试日志
#[macro_export]
macro_rules! plugin_debug {
    ($plugin:expr, $($arg:tt)*) => {
        if $crate::symbio_core::is_logger_initialized() {
            tracing::debug!(plugin = %$plugin, $($arg)*);
        } else {
            eprintln!("[{} DEBUG] {}", $plugin, format_args!($($arg)*));
        }
    };
}

/// 插件级别警告日志
#[macro_export]
macro_rules! plugin_warn {
    ($plugin:expr, $($arg:tt)*) => {
        if $crate::symbio_core::is_logger_initialized() {
            tracing::warn!(plugin = %$plugin, $($arg)*);
        } else {
            eprintln!("[{} WARN] {}", $plugin, format_args!($($arg)*));
        }
    };
}

/// 插件级别错误日志
#[macro_export]
macro_rules! plugin_error {
    // 支持带格式化参数的形式: plugin_error!("name", "fmt {}", arg)
    ($plugin:expr, $fmt:literal, $($arg:tt)*) => {
        if $crate::symbio_core::is_logger_initialized() {
            tracing::error!(plugin = %$plugin, $fmt, $($arg)*);
        } else {
            eprintln!("[{} ERROR] {}", $plugin, format_args!($fmt, $($arg)*));
        }
    };
    // 支持单表达式形式 (兼容旧代码): plugin_error!("name", format!(...)) 或 plugin_error!("name", "msg")
    ($plugin:expr, $err:expr) => {
        if $crate::symbio_core::is_logger_initialized() {
            tracing::error!(plugin = %$plugin, error = ?$err);
        } else {
            eprintln!("[{} ERROR] {}", $plugin, $err);
        }
    };
}
