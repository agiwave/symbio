//! 世界端口的注册面转发（`D-440` ②）。
//!
//! 端口契约整面住 `symbio_sdk::capability::port`（读/校验半 `D-404` / `D-414` 先搬，
//! 注册面与执行契约 `D-440` ② 随行）：门面是架构词汇层，自有实现只许降（FA-001），
//! 而端口契约不认识任何世界词，与 sdk 其余读口同一条边界。这里只转发，插件与宿主
//! 的既有导入路径不变；`PortDecl`、派发 / 等待两行模板、`Call{port,id,attempt,via}`
//! 的幂等键与记账住 `loop`。
//!
//! 语义细则（端口集开放 / 提供者随能力表走 / 判重与缺席）见 sdk 侧模块文档。

pub use symbio_sdk::capability::port::{capability_port_check, capability_port_of};
pub use symbio_sdk::capability::port::{check_port_name, port_family, port_row_name};
pub use symbio_sdk::capability::port::{port_rows, AuthzVerdict, PendingPrompt};
pub use symbio_sdk::capability::port::{PortCapabilities, PortErrorClass, PortExecContext};
pub use symbio_sdk::capability::port::{PortExecHandles, PortExecutor, PortFact};
pub use symbio_sdk::capability::port::{PortFactEmit, PortFactReader, PortFactWriter};
pub use symbio_sdk::capability::port::{PortInput, PortOutcome, PortPayload, PortProvider};
pub use symbio_sdk::capability::port::{PortReading, PortReply, PORT_FAMILY};

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
