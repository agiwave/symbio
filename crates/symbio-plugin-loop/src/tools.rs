//! 工具执行域（tools）：模型发出的工具调用**从分发到结果落库的全链路**（session 域内）。
//!
//! 域内三个子模块：
//!
//! - `tool_executor` —— 执行编排：单笔债的通用入口（`execute_one`）、
//!   批量分发与结果广播（`process_tool_calls_in_env`）。它是工具结果节点的
//!   **唯一写入者**，也是编排层与上下文治理层取用工具机制的入口。
//! - `dispatch` —— `process_tool_calls_in_env` 的**执行阶段分层**：
//!   `batch` 逐条准备 / 执行 / 收口，`gates` 只留拒止骨架与外部执行的中性输入；
//!   编排入口仍是 `tool_executor` 里同名函数，本层只是它的落点，不改门面。
//!   「没跑到」的**成因词表**（`ABORTED` / `NOT_EXECUTED`）也经这条门面交回，
//!   收口由 `tool_dispatch` 写 `failed{why}`。
//!
//! 单条结果的体积上限（守卫与归档滚动）与**产物格落格**都随执行体迁到
//! `symbio-plugin-tool`；本域只消费执行体交回的**已守卫文本**与守卫元数据
//! （P1b，DE-047 D-308）。
//!
//! 生命周期钩子（`fire_hook`）住宿主 `symbio_plugin_api::hooks`：session 派发前与 loop 轮内都触发它，
//! 两边共用同一条路由助手（[D-086](../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §6）。
//!
//! `heartbeat` 工具**不在这里**：它随调度一起切出为 `crates/symbio-plugin-heartbeat`
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

pub use self::dispatch::{is_unanswered, NOT_EXECUTED};
// 中止的成因词：与 `NOT_EXECUTED` 同属「没跑到」的词表，跨域取值只经这条门面。
// 生产面没有引用点（端口关账读的是结果节点上的 `failure_kind` 本身），
// 非测试构建下允许无引用——同 core 事件字典的做法（见 `symbio_sdk` 的 `lib.rs`）。
#[allow(unused_imports)]
pub use self::dispatch::ABORTED;
// 外部执行闸门的判据已搬进 `symbio-plugin-tool` 的执行体；环只留授权 / 成本的中性
// 输入（`GateInputs`），不预判、不拒发（DE-039 §6，DE-047 D-311）。
// 生产入口（带执行信封）：分发方在派发时把轮号 / 根节点 / 产物闸结论 / 写口
// 装进 [`RoundEnv`]，执行体据此落产物格（P1b，DE-047 D-308）。
pub(crate) use self::tool_executor::{land_outcome_async, process_tool_calls_in_env, RoundEnv};
