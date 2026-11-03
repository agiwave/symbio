//! 驱动器 → 会话的**动作面客户端**（`port=session` 的 `session-call` / `session-call-unit` 行）。
//!
//! 会话动作（转写 / 压缩 / 补充）的 owner 全是 session。驱动器不持会话引擎、也不认识
//! 它的实现，只按端口名解析提供者，把「一次动作」包成两行契约之一：
//!
//! | 行 | 载荷（中性键） | 回体 |
//! |---|---|---|
//! | `session-call` | `{ row, op, payload }` | `{ data }` |
//! | `session-call-unit` | `{ row, op, payload }` | `{}` |
//!
//! `op` 仍取 [`run_ops`] 那个装配期冻结的闭集（`D-094`）：端口不为每个动作加一行。
//! 授权、载荷校验、回体装配都在提供者一侧（session 的 `run_at` / 端口执行体）。
//!
//! 本模块只是**客户端**：取提供者、构造通用执行上下文、搬载荷与回体。端口缺席或
//! 提供者没有执行面都**响亮失败**（不 panic、不退回第二份读法）。

use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::{PluginError, PluginInvokeRequest};
use symbio_plugin_api::port::{
    PortExecContext, PortFactEmit, PortInput, PortOutcome, PortProvider,
};

/// 端口名（与 `request` 事实载荷里的 `port` 同一个值，`DE-047` I-6.1）。
const PORT: &str = "session";
/// 行判别键（中性键）。
const ROW_KEY: &str = "row";
/// 有回体的动作行。
const ROW_CALL: &str = "session-call";
/// 只回执的动作行。
const ROW_CALL_UNIT: &str = "session-call-unit";

/// 驱动器对会话动作面的**窄端口**：两条行契约各一个方法。
#[async_trait]
pub(crate) trait SessionPort: Send + Sync {
    /// `session-call`：载荷与回体都按 `run_ops` 词表的请求结构体定。
    async fn call(&self, op: &str, payload: Value) -> Result<Value, PluginError>;

    /// `session-call-unit`：只回执。
    async fn call_unit(&self, op: &str, payload: Value) -> Result<(), PluginError>;
}

/// 发一个**有回体**的动作（`session-call` 行）。
pub(crate) async fn call<T: DeserializeOwned>(
    port: &dyn SessionPort,
    op: &str,
    payload: impl Serialize,
) -> Result<T, PluginError> {
    let data = port.call(op, to_payload(op, payload)?).await?;
    serde_json::from_value(data)
        .map_err(|e| PluginError::InternalError(format!("会话动作回体不合法（{op}）：{e}")))
}

/// 发一个**无回体**的动作（`session-call-unit` 行）。
pub(crate) async fn call_unit(
    port: &dyn SessionPort,
    op: &str,
    payload: impl Serialize,
) -> Result<(), PluginError> {
    port.call_unit(op, to_payload(op, payload)?).await
}

/// 载荷序列化：失败只能是调用点传了不可序列化的值，按 `Internal` 出声。
fn to_payload(op: &str, payload: impl Serialize) -> Result<Value, PluginError> {
    serde_json::to_value(payload)
        .map_err(|e| PluginError::InternalError(format!("动作载荷序列化失败（{op}）：{e}")))
}

/// 经能力表解析 `port=session` 提供者、按行契约派发的实现。
///
/// 每次调用都现场取一次提供者（插件可能在运行期重新装配）：取提供者是一次读，
/// 不是热路径。
pub(crate) struct PortSession {
    host_ctx: Arc<dyn PluginInvokeRequest>,
}

impl PortSession {
    pub(crate) fn new(host_ctx: Arc<dyn PluginInvokeRequest>) -> Self {
        Self { host_ctx }
    }

    async fn invoke(
        &self,
        row: &'static str,
        op: &str,
        payload: Value,
    ) -> Result<PortOutcome, PluginError> {
        let provider = self.provider().await?;
        let executor = provider
            .executor()
            .ok_or_else(|| PluginError::NotFound("会话端口提供者没有交执行面".to_string()))?;
        let exec = PortExecContext {
            deadline_ms: None,
            abort: ExecAbortSignal::new(),
            idempotency_key: format!("{row}:{op}"),
            sink: ExecEventSink::Null,
            emit: PortFactEmit::silent(),
            ctx: Some(self.host_ctx.clone()),
            subject: 0,
            group: String::new(),
            origin: None,
            permitted: true,
            deny_reason: String::new(),
            scratch_dir: None,
        };
        let input = PortInput {
            port: PORT,
            payload: json!({ ROW_KEY: row, "op": op, "payload": payload }),
        };
        let outcome = executor.execute(exec, input).await;
        if let Some((_, why)) = outcome.error {
            return Err(PluginError::InternalError(format!(
                "会话动作失败（{op}）：{why}"
            )));
        }
        Ok(outcome)
    }

    async fn provider(&self) -> Result<Arc<dyn PortProvider>, PluginError> {
        super::port::provider_of(&self.host_ctx, PORT)
            .await
            .ok_or_else(|| PluginError::NotFound("会话端口没有可用提供者".to_string()))
    }
}

#[async_trait]
impl SessionPort for PortSession {
    async fn call(&self, op: &str, payload: Value) -> Result<Value, PluginError> {
        let outcome = self.invoke(ROW_CALL, op, payload).await?;
        Ok(outcome
            .facts
            .first()
            .and_then(|reply| reply.get("data"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn call_unit(&self, op: &str, payload: Value) -> Result<(), PluginError> {
        self.invoke(ROW_CALL_UNIT, op, payload).await.map(|_| ())
    }
}
