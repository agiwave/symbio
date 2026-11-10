//! 工具行的端口执行体桥（`D-440` ②）：**工具就是端口**。
//!
//! 收集器在工具行入表时同步派生一条 `PORT_FAMILY` 行（端口名 = 行名，见
//! [`tool_port_row`]），执行体是本模块的桥：把 [`PortInput`] 的载荷当作调用参数、
//! 经 `capability_invoke`（信封 ↔ 结果换算的唯一收口）调已过闸的能力，再把结果按
//! [`RESULT_KEY`] 约定装回 [`PortOutcome`]。派发侧（`plugin-tool`）经
//! [`symbio_plugin_api::port_executor_of`] 按端口名取到的就是它——行上不再持
//! 执行对象，`Capability` 退场（③）时工具插件的执行体直接实现 [`PortExecutor`]，
//! 本桥随行。
//!
//! 闸门在注册收口点已裹（`tool_visitor.rs` 的 `ApprovalGate`），桥调的是**已过闸**
//! 的那一个：判定只有 `approval_gate` 一份，桥不复判。闸门拦下的停等载荷是普通
//! 结果形态（`failure_kind` 在载荷里），原样经 [`RESULT_KEY`] 交回，派发侧的
//! pending 机制照常工作。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use symbio_kernel::capability::{Capability, CapabilityRow};
use symbio_kernel::plugin::PluginInvokeRequestExt;
use symbio_plugin_api::port::PORT_FAMILY;
use symbio_plugin_api::{
    capability_invoke, port_row_name, AuthzVerdict, PortErrorClass, PortExecContext, PortExecutor,
    PortInput, PortOutcome, PortPayload, PortProvider, RESULT_KEY,
};

/// 工具行派生端口的提供者 id：收集器不认识插件名（它只看得见行），注册侧用它
/// 占位——行名形状 `<port:{工具名}>:tool-row`。判重按**端口名**（工具行名全表唯一，
/// 端口名随之唯一），id 不参与判定。
const TOOL_ROW_PROVIDER: &str = "tool-row";

/// 一个工具行的端口提供者：单端口（端口名 = 行名），执行体是能力桥。
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

/// 能力桥执行体：`PortInput.payload` 即调用参数；结果（含闸门拦下的停等载荷）
/// 按 [`RESULT_KEY`] 交回。
struct CapabilityBridge {
    cap: Arc<dyn Capability>,
}

#[async_trait]
impl PortExecutor for CapabilityBridge {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(request) = exec.ctx else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "工具行执行体没有调用方上下文，无法拆信封",
            );
        };
        let _ = request.set_payload(input.payload);
        match capability_invoke(self.cap.as_ref(), request).await {
            Ok(payload) => {
                let value = payload.get::<Value>().unwrap_or(Value::Null);
                PortOutcome {
                    authz: AuthzVerdict::Authorized,
                    facts: vec![json!({ RESULT_KEY: value })],
                    error: None,
                    reply: Default::default(),
                }
            }
            Err(e) => PortOutcome::failed(PortErrorClass::NonRetryable, e.to_string()),
        }
    }
}

/// 由一条**已过闸**的工具能力派生端口行：端口名 = 行名（工具的线上全名），执行体
/// 裹同一个能力实例。与 `port_rows` 同一条注册形状（`multi` 族、`active = false`），
/// 只是提供者由行派生、不是插件申报。
pub fn tool_port_row(cap: Arc<dyn Capability>, name: &str) -> CapabilityRow {
    let executor: Arc<dyn PortExecutor> = Arc::new(CapabilityBridge { cap });
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
