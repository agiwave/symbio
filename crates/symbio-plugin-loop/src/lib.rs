//! 单环驱动器插件（`loop`，[D-086] §8）。
//!
//! 一轮对话的执行引擎：上下文装配、轮首判决、工具循环、收束与派生事实。
//! 它**不拥有**会话状态——转写、事实源、压缩熔断、用户补充队列都在 session；
//! 驱动器经 [`session_port`]（`port=session` 的 `session-call` / `session-call-unit` 行）读写它们。
//!
//! 入口只有一个：session 派发 `loop/<sid>` 的 `run_turn`（见 [`provider`]）。
//!
//! 决策 D-086 见 [D-086](../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "D-086")

mod chat_loop;
mod context;
// 控制面声明集：义务环自己消费的事实与全部视图。新增视图 / 事实 / 字段改这里，
// 不改内核（`symbio_kernel::decl` 只留协议五格与装载机制）。
mod control_decl;
mod derived_facts;
// 义务环的行表：控制面经 `owes` 问表，判据在数据里。
mod loop_rows;
mod memory_events;
mod message_build;
mod plugin;
// 端口的执行半边：协议事实的唯一写方 + 租约看护。
mod port;
// 本插件的投影（原 core 的 consolidate，DE-025 D-221）：唯一读方是
// `memory_events`，判定跟着调用方走。
mod projections;
mod provider;
// 记忆检索（`recall` 投影与排序内核）自 L2b 起住 `crates/symbio-plugin-memory` 的
// `recall` 子模块（DE-026 D-264）：控制面只留可见域收放（`memory_events`）与读口。
// 轮中汇报那一行的落点与能力口（DE-015 的 D-163）。
mod report_exec;
mod session_port;
mod stage_events;
mod tool_dispatch;
mod tool_exec;
mod turn_exec;
mod turn_runner;

#[cfg(test)]
mod test_support;
