//! 工具执行域（tools）：模型发出的工具调用**从分发到结果落库的全链路**（session 域内）。
//!
//! 域内三个子模块：
//!
//! - `tool_executor` —— 执行编排：单工具路由与执行（`execute_tool_async`）、
//!   批量分发与结果广播（`process_tool_calls_async`）、生命周期钩子（`fire_hook`）、
//!   「未执行」结果的统一写法（`apply_not_executed`）。它是工具结果节点的
//!   **唯一写入者**，也是编排层与上下文治理层取用工具机制的入口。
//! - `tool_result_guard` —— 结果守卫：单条结果进存储前的体积上限（`guard_tool_result`
//!   与归档滚动），以及**不写存档**的请求视图级摘要（`summarize_tool_result` /
//!   `summarize_head_tail`）——上下文治理层的工具淡化与 token 超预算裁剪共用同一份
//!   头尾摘要语义，因此它必须暴露给域外，而不是私藏在执行路径里。
//!
//! `heartbeat` 工具**不在这里**：它随调度一起切出为 `plugins/heartbeat`
//! （[ADR-077](../../../../docs/010-adr/077.md "ADR-077")），而本域是分发与执行基础设施。
//!
//! ## 共享面走门面，不深潜
//!
//! 域内互引经 `super::<兄弟子模块>`；**跨域**（编排层 / 上下文治理层 / 插件装载）
//! 取用一律走本文件的 `pub use` 门面。session 的其它子模块不得深潜
//! `tools::<子模块>::X`——子模块一律 `mod`（私有），跨域只看得见这里列出来的项；
//! 需要新增跨域项时改的是这张门面，而不是放宽子模块可见性。

mod tool_executor;
mod tool_result_guard;

pub use self::tool_executor::{
    apply_not_executed, execute_tool_async, fire_hook, process_tool_calls_async, Delegation,
    TaskDeclaration, TaskItem, TaskStatus,
};
pub use self::tool_result_guard::{summarize_head_tail, summarize_tool_result};
