//! 结构化日志系统
//!
//! 使用 tracing 提供统一的日志接口，支持级别过滤和字段附加。
//!
//! ## 行内标签约定
//!
//! 日志行的前缀由两段组成，各司其职：
//!
//! - `plugin` 字段（输出为 `[<plugin> <LEVEL>]`）标识**拥有者**——哪来的；
//! - 正文开头的 `[Tag]` 标识**阶段**——同一插件内的哪一段。
//!
//! **什么时候需要 `[Tag]`**：当**一个插件会打出多个阶段**时才需要。典型是 `session`：
//! 它同时输出会话生命周期（`[Session]`）、轮次（`[Turn]`）、工具批（`[Tool]`）、
//! 转写帧（`[Transcript]`）、流消费（`[Consume]`）、压缩（`[Compress]`）、恢复（`[Resume]`）
//! ——没有标签就只能靠措辞猜。反之，单一用途的插件（`mcp` / `model` / `home` …）
//! 的 plugin 字段已经是它唯一的标签，正文不必再套一层。
//!
//! **写法**：`[Tag] 中文短句`。不要用 `>>>` / `--- ... ---` 之类的装饰，也不要留
//! 英文散句——它们让同一插件的输出看起来像来自不同系统。历史遗留的不带标签行按此
//! 约定在改动时顺带收敛（不做纯机械批量改名）。
//!
//! ## 高频日志的分级
//!
//! 同一件事**逐帧 / 逐次**重复的日志（流式增量、每帧状态、每轮扫描）默认沉到 `DEBUG`，
//! `INFO` 只留**骨架**——开始、结束、异常、等待人介入。参照 `session` 的转写核心日志
//! （`plugins/session/docs/node-state-streaming.md` §10.5）：同一场景默认输出 14 行、
//! `debug` 19 行，多出来的正是过程量。
//!
//! ## 级别与过滤
//!
//! - 装了 subscriber（App）：由 `EnvFilter`（`RUST_LOG`）过滤。
//! - 没装 subscriber（CLI）：宏退回 `eprintln!`，由 [`MIN_LEVEL`] 闸门过滤，
//!   默认 INFO（debug 静默）；`--verbose` 或 `SYMBIO_LOG=<级别>` 放开。

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// 全局日志订阅器状态
static LOGGER_INITIALIZED: OnceLock<()> = OnceLock::new();

/// 级别序数：越小越宽松（与 `EnvFilter` 的 level 语义一致，便于比较）。
pub const LEVEL_DEBUG: u8 = 0;
pub const LEVEL_INFO: u8 = 1;
pub const LEVEL_WARN: u8 = 2;
pub const LEVEL_ERROR: u8 = 3;

/// 无订阅器路径的最低输出级别（默认 INFO）。
///
/// ## 为什么需要这道闸门
///
/// 日志宏有两条落地路径：装了 tracing subscriber 时由它的 `EnvFilter` 过滤；
/// **没装**时退回 `eprintln!`。后者**没有过滤器**——若不设闸门，`debug` 会无条件
/// 打到 stderr，把启动期的机械细节（如「正在构造子插件 …」每插件一行）混进
/// 用户可见输出。CLI 正是这条路径（刻意不装 subscriber，让 stdout 保持纯净）。
///
/// 有订阅器时本闸门不参与（过滤交给 `EnvFilter`），因此对 App 行为零影响。
static MIN_LEVEL: AtomicU8 = AtomicU8::new(LEVEL_INFO);

/// 设置无订阅器路径的最低输出级别（越低越宽松）。见 [`MIN_LEVEL`]。
pub fn set_min_level(level: u8) {
    MIN_LEVEL.store(level, Ordering::Relaxed);
}

/// 当前最低输出级别。
pub fn min_level() -> u8 {
    MIN_LEVEL.load(Ordering::Relaxed)
}

/// 解析级别名（`debug` / `info` / `warn` / `error`，大小写与首尾空白不敏感）。
///
/// `trace` 归到 `debug`、`off` 归到 `error`——本系统只有四档，不需要更细的映射。
pub fn parse_level(name: &str) -> Option<u8> {
    match name.trim().to_ascii_lowercase().as_str() {
        "debug" | "trace" => Some(LEVEL_DEBUG),
        "info" => Some(LEVEL_INFO),
        "warn" | "warning" => Some(LEVEL_WARN),
        "error" | "off" => Some(LEVEL_ERROR),
        _ => None,
    }
}

/// 供日志宏判定的闸门：有订阅器时一律放行，否则比对本条级别与 [`MIN_LEVEL`]。
#[inline]
pub fn level_enabled(level: u8) -> bool {
    if is_logger_initialized() {
        return true;
    }
    level >= min_level()
}

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
        } else if $crate::symbio_core::level_enabled($crate::symbio_core::LEVEL_INFO) {
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
        } else if $crate::symbio_core::level_enabled($crate::symbio_core::LEVEL_DEBUG) {
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
        } else if $crate::symbio_core::level_enabled($crate::symbio_core::LEVEL_WARN) {
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
        } else if $crate::symbio_core::level_enabled($crate::symbio_core::LEVEL_ERROR) {
            eprintln!("[{} ERROR] {}", $plugin, format_args!($fmt, $($arg)*));
        }
    };
    // 支持单表达式形式: plugin_error!("name", format!(...)) 或 plugin_error!("name", "msg")
    ($plugin:expr, $err:expr) => {
        if $crate::symbio_core::is_logger_initialized() {
            tracing::error!(plugin = %$plugin, error = ?$err);
        } else if $crate::symbio_core::level_enabled($crate::symbio_core::LEVEL_ERROR) {
            eprintln!("[{} ERROR] {}", $plugin, $err);
        }
    };
}

#[cfg(test)]
#[path = "logger.test.rs"]
mod tests;
