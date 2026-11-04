//! 假插件验收：只在本 crate 里定义一个端口 `demo` 与一笔债。
//!
//! 验收判据：环 / `plugin-api` / `kernel` 三个 crate 零改动，跑通
//! 「环选债 → 透传字符串 → 宿主查到执行体 → 执行体还事实 → 环配对」。

mod plugin;
mod port;

pub use plugin::DemoPlugin;
