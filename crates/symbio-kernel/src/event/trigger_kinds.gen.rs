//! 触发方式的登记表 —— **由 `scripts/facts/trigger-kinds.mjs` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 `PLUGIN.yml` 的 `trigger_kinds:` 行（谁产出触发，
//! 谁声明它写得出哪些 kind）。消费方只判**成员资格**——加一种触发方式 = 产出方在
//! 自己目录里加一个取值，消费方零 diff。
//!
//! 未登记的 kind 在写臂**当场拒**（不静默降级成普通用户消息）：拼错的触发声明若被
//! 当没看见，那一轮就不再是自主的，而事实源里少的那几格没有任何东西会补。

/// 登记过的触发方式：写体 `meta.trigger.kind` 只认这里面的取值。
#[rustfmt::skip]
pub const EVENT_TRIGGER_KINDS: &[&str] = &["scheduled"];
