//! 工具执行域（tools）：模型发出的工具调用**从分发到结果落库的全链路**（session 域内）。
//!
//! 域内三个子模块：
//!
//! - `tool_executor` —— 执行编排：单工具路由与执行（`execute_tool_async`）、
//!   批量分发与结果广播（`process_tool_calls_async`）。它是工具结果节点的
//!   **唯一写入者**，也是编排层与上下文治理层取用工具机制的入口。
//! - `dispatch` —— `process_tool_calls_async` 的**执行阶段分层**：
//!   ① 批首闸门读数 ② 逐条执行 ③ 未执行收口；编排入口仍是 `tool_executor` 里同名函数，
//!   本层只是它的三段落点，不改门面。收口段（`unfinished`）同时是「没跑到」的
//!   **成因词表**（`ABORTED` / `NOT_EXECUTED`）与结果子节点的落法所在。
//! - `tool_result_guard` —— 结果守卫：单条结果进存储前的体积上限（`guard_tool_result`
//!   与归档滚动）。它是**唯一**的体积裁剪点：请求体不再做淡化 / 骨架化改写
//!   （[D-078](../../symbio-plugin-session/README.md#d-078-请求视图三层淡化退役请求体是事实网格的纯投影注入段只追加不改写 "crates/symbio-plugin-session/src/README")）。
//!
//! 生命周期钩子（`fire_hook`）住宿主 `symbio_plugin_api::hooks`：session 派发前与 loop 轮内都触发它，
//! 两边共用同一条路由助手（[D-086](../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §6）。
//!
//! `heartbeat` 工具**不在这里**：它随调度一起切出为 `plugins/heartbeat`
//! （[D-077](../../symbio-plugin-heartbeat/README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README")），而本域是分发与执行基础设施。
//!
//! ## 共享面走门面，不深潜
//!
//! 域内互引经 `super::<兄弟子模块>`；**跨域**（编排层 / 上下文治理层 / 插件装载）
//! 取用一律走本文件的 `pub use` 门面。session 的其它子模块不得深潜
//! `tools::<子模块>::X`——子模块一律 `mod`（私有），跨域只看得见这里列出来的项；
//! 需要新增跨域项时改的是这张门面，而不是放宽子模块可见性。

mod dispatch;
mod tool_executor;
mod tool_result_guard;

pub use self::dispatch::unfinished::{is_unanswered, NOT_EXECUTED};
// 中止的成因词：与 `NOT_EXECUTED` 同属「没跑到」的词表，跨域取值只经这条门面。
// 生产面没有引用点（端口关账读的是结果节点上的 `failure_kind` 本身），
// 非测试构建下允许无引用——同 core 事件字典的做法（见 `symbio_core::mod.rs`）。
#[allow(unused_imports)]
pub use self::dispatch::unfinished::ABORTED;
// 外部执行闸门（S09）：判据住在 `gates`（唯一生产消费方是本域的 `external_execution`，
// D-190）；`derived_facts` 的熔断写方经本门面取用，不深潜子模块。
pub(crate) use self::dispatch::gates::CircuitBreaker;
// 测试面（`tool_executor.test.rs`）要按结论比对；生产侧只问「是不是 Allow」。
#[cfg(test)]
pub(crate) use self::dispatch::gates::GateDecision;
pub use self::tool_executor::process_tool_calls_async;
