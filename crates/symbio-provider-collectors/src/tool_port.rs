//! 工具行的端口执行体桥（`D-440` ②）：**工具就是端口**。
//!
//! 收集器在工具行入表时同步派生一条 `PORT_FAMILY` 行（端口名 = 行名，见
//! [`tool_port_row`]），执行体是收集器裹好闸门的 [`GatedExecutor`]（审批判定是
//! 执行的第一段，见 `approval.rs`）：`PortInput` 的载荷即调用参数，出口按
//! [`RESULT_KEY`] 约定装回 [`PortOutcome`]。派发侧（`plugin-tool`）经
//! [`symbio_plugin_api::port_executor_of`] 按端口名取到的就是它——行上不再持
//! 执行对象，`Capability` 退场（③-B）时工具插件的执行体直接实现 [`PortExecutor`]，
//! 本提供者形状随行。

use std::sync::Arc;

use symbio_kernel::capability::CapabilityRow;
use symbio_plugin_api::port::PORT_FAMILY;
use symbio_plugin_api::{port_row_name, PortExecutor, PortPayload, PortProvider};

/// 工具行派生端口的提供者 id：收集器不认识插件名（它只看得见行），注册侧用它
/// 占位——行名形状 `<port:{工具名}>:tool-row`。判重按**端口名**（工具行名全表唯一，
/// 端口名随之唯一），id 不参与判定。
const TOOL_ROW_PROVIDER: &str = "tool-row";

/// 一个工具行的端口提供者：单端口（端口名 = 行名），执行体自带闸门。
pub struct ToolRowPort {
    port: String,
    executor: Arc<dyn PortExecutor>,
}

impl PortProvider for ToolRowPort {
    fn ports(&self) -> Vec<String> {
        // 动态端口名（数据，来自入表的行名）：不走 `vec![字面量]` 的静态声明形状，
        // 端口面审计按静态声明解析时自然略过本类型。
        Vec::from([self.port.clone()])
    }

    fn provider_id(&self) -> &str {
        TOOL_ROW_PROVIDER
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::clone(&self.executor))
    }
}

/// 由一条**已过闸**的工具执行体派生端口行：端口名 = 行名（工具的线上全名）。
/// 与 `port_rows` 同一条注册形状（`multi` 族、`active = false`），只是提供者由行
/// 派生、不是插件申报。
pub fn tool_port_row(executor: Arc<dyn PortExecutor>, name: &str) -> CapabilityRow {
    CapabilityRow::new(
        PORT_FAMILY,
        port_row_name(name, TOOL_ROW_PROVIDER),
        PortPayload::new(Arc::new(ToolRowPort {
            port: name.to_string(),
            executor,
        })),
        false,
    )
}
