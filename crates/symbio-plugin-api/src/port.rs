//! 世界端口的收集面转发（`D-440` ② / `D-468`）。
//!
//! 端口契约整面住 `symbio_sdk::capability::port`（读/校验半 `D-404` / `D-414` 先搬，
//! 注册面与执行契约 `D-440` ② 随行，收集容器 `D-468`）：门面是架构词汇层，自有实现只许
//! 降（FA-001），而端口契约不认识任何世界词，与 sdk 其余读口同一条边界。这里只转发，
//! 插件与宿主的既有导入路径不变；`PortDecl`、派发 / 等待两行模板、`Call{port,id,attempt,via}` 的幂等键与记账住 `loop`。
//!
//! 语义细则（端口集开放 / 提供者随装配走 / 判重与缺席）见 sdk 侧模块文档。

pub use symbio_sdk::capability::port::PortReply;
pub use symbio_sdk::capability::port::{
    check_port_name, register_ports, scoped_port_name, PortBindings, PORT_BINDINGS, PORT_SCOPE,
};
pub use symbio_sdk::capability::port::{port_result, PortExecHandles, PortExecutor, PortFact};
pub use symbio_sdk::capability::port::{AuthzVerdict, PendingPrompt, RESULT_KEY};
pub use symbio_sdk::capability::port::{PortCapabilities, PortErrorClass, PortExecContext};
pub use symbio_sdk::capability::port::{PortFactEmit, PortFactReader, PortFactWriter};
pub use symbio_sdk::capability::port::{PortInput, PortOutcome, PortProvider};

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
