//! 单环驱动器插件（`loop`，[D-086] §8）。
//!
//! 一轮对话的执行引擎：上下文装配、轮首判决、工具循环、收束与派生事实。
//! 它**不拥有**会话状态——转写、事实源、压缩熔断、用户补充队列都在 session；
//! 驱动器经 [`session_port`]（VDFS 地址面 `session/<sid>/_run/<op>`）读写它们。
//!
//! 入口只有一个：session 派发 `loop/<sid>` 的 `run_turn`（见 [`provider`]）。
//!
//! 决策 D-086 见 [D-086](../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "D-086")

mod chat_loop;
mod context;
mod derived_facts;
// 轮首 host 的执行体与派发点（DE-015 的 D-159）：与 `turn_exec` 同形状，借的面不同。
mod head_exec;
// 义务环的行表：控制面经 `owes` 问表，判据在数据里。
mod loop_rows;
mod memory_events;
mod message_build;
mod plugin;
// 端口的执行半边：协议事实的唯一写方 + 租约看护。
mod port;
mod provider;
// 批次边界 host 的执行体与派发点（DE-015 的 D-163）：轮中汇报住这一台。
mod report_exec;
mod session_port;
mod skill_events;
// 外环 host 的执行体与派发点（DE-015 的 D-168）：一个幕的 `turn` 往返
// 拆成「宣布」与「派出」两个环步——与前两台同形状，借的面不同。
mod stage_events;
mod stage_exec;
mod task_events;
mod tokenizer;
mod tool_dispatch;
mod tools;
mod turn_exec;
mod turn_runner;
mod turn_session;

#[cfg(test)]
mod test_support;
