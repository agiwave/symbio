//! 工具执行域（tool_exec）：模型发出的工具调用**从派发到结果落盘的全链路**。
//!
//! P2-5b 合并后，原 `tools::{dispatch,tool_executor}` 的分层收成五个子模块：
//! `execution` 逐笔驱动（准备 → 执行 → 收口），`envelope` 只装中性信封与载荷，
//! `decls` / `effects` / `gate_inputs` 是环侧声明、效果与闸门读数。
//!
//! 单条结果的体积上限（守卫与归档滚动）与**产物格落格**都随执行体迁到
//! `symbio-plugin-tool`；本域只消费执行体交回的**已守卫文本**与守卫元数据
//! （P1b，DE-048 D-308）。环不认识工具世界：载荷键只放中性纯值（D-316）。

mod decls;
mod effects;
mod envelope;
mod execution;
mod gate_inputs;

pub(crate) use self::envelope::RoundEnv;
#[allow(unused_imports)]
pub(crate) use self::execution::{
    is_unanswered, land_outcome_async, process_tool_calls_in_env, ABORTED, NOT_EXECUTED,
};
