//! 词汇层的声明读法 —— 「这个名字属于哪一格」。
//!
//! 冻结机制住 `symbio_sdk::declarations`（链接期收集、装配期校验、进程内冻结一次）：
//! 那是宿主问表的机制。本文件只留**读法**：先问冻结表，未命中再问清单生成物
//! [`events::SHARED_EVENTS`]；三份表都不认识的名字按「网格之外」处理（fail-closed）。
//!
//! 回落表为什么不跟着搬：`events.rs` 是各 owner 声明经生成器投影的共享词汇面，而内核与
//! sdk 的生产代码**零事件名字面量**（CS-008 / CP-004，`D-415`）。读法因此与生成物同住
//! 词汇层。

use symbio_kernel::event::{Entity, EventDecl, Verb};

/// 判据的词汇面，**没有「内核表」这一档**（DE-024 D-203 裁决三）。
///
/// 冻结表缺席时用清单生成物 [`events::SHARED_EVENTS`]：它同样是 owner 交的
/// （`PLUGIN.yml` 的 `shared_events`），只是范围窄——只有跨插件共享的那些名字。
/// 内核不替任何人记名字，所以没有可回退的第三张表：三份表都不认识的名字按
/// 「网格之外」处理（fail-closed）。
pub fn vocab() -> &'static [EventDecl] {
    symbio_sdk::declarations::declared_vocab().unwrap_or(crate::events::SHARED_EVENTS)
}

/// 这个名字的格位：先问冻结表，未命中再问清单生成物，都不认识 ⇒ `None`
/// （调用方按「网格之外」拒绝，fail-closed）。
pub fn declared_event_cell(name: &str) -> Option<(Entity, Verb)> {
    vocab().iter().find(|e| e.name == name).map(|e| e.cell)
}
