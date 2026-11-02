//! model 端口的世界提供者（D-298 M1）：解析与适配器构造收在 model 插件一侧。
//!
//! 环（loop）只按端口名取——provider 怎么解析（严格查 `preferred` → 落回当次
//! 生效者）、适配器怎么构造，都是这里的私事。`ModelPort` 在 traverse 里随能力
//! 目录一起装：生效者缺席 ⇒ 不注册端口行 ⇒ 环在调用点响亮失败（落失败事实
//! 并中止），而不是在这里猜一个别的模型。

use std::sync::Arc;

use symbio_plugin_api::chat_node::ChatNode;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::ports;
use symbio_plugin_api::{
    CapabilityMeta, CapabilityRow, ExecAbortSignal, ExecEnv, LlmAdapter, ModelFinishReason,
    ModelProvider, PluginError, ProviderLlmAdapter,
};

/// `port.model` 的提供者：当次生效者 + 完整目录（严格解析用）。
///
/// 目录 = 全部已启用的 provider（与能力目录同一批 `BoundProvider`）；生效者
/// 必然是目录一员（不在 ⇒ traverse 不注册本行，见模块文档）。`preferred`
/// 查目录不中生 ⇒ 落回生效者——降级方向只有这一处定义。
pub(crate) struct ModelPort {
    active: Arc<dyn ModelProvider>,
    catalog: Vec<Arc<dyn ModelProvider>>,
}

impl ModelPort {
    pub(crate) fn new(
        active: Arc<dyn ModelProvider>,
        catalog: Vec<Arc<dyn ModelProvider>>,
    ) -> Self {
        Self { active, catalog }
    }

    /// 注册行（本提供者只服务一个端口，行数恒为 1）。
    pub(crate) fn rows(self: Arc<Self>) -> Vec<CapabilityRow> {
        symbio_plugin_api::port::port_rows(&(self as Arc<dyn PortProvider>))
    }
}

impl PortProvider for ModelPort {
    fn ports(&self) -> &'static [&'static str] {
        &[ports::MODEL]
    }

    fn provider_id(&self) -> &str {
        self.active.provider_id()
    }

    fn llm_round_trip(
        &self,
        preferred: Option<&str>,
        system_prompt: &str,
        abort: ExecAbortSignal,
    ) -> Option<Arc<dyn LlmAdapter>> {
        let provider = preferred
            .and_then(|id| self.catalog.iter().find(|p| p.provider_id() == id))
            .unwrap_or(&self.active);
        Some(Arc::new(ProviderLlmAdapter::for_turn(
            provider.clone(),
            system_prompt,
            abort,
        )))
    }

    fn model(&self) -> Option<Arc<dyn ModelProvider>> {
        Some(self.active.clone())
    }

    /// 通用执行面（`DE-047` D-305 裁决一）：一次往返的请求构造、流式帧、工具调用
    /// 累计、用量与计价、终止判定都在执行体里；环只把 `request` 载荷原样交进来。
    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(ModelExecutor {
            active: self.active.clone(),
            catalog: self.catalog.clone(),
        }))
    }
}

/// `request(port=model)` 的载荷（行契约由 model 插件自己持有，环不认识这些键）。
#[derive(Debug, serde::Deserialize)]
struct RoundTripRequest {
    /// 首选 provider id；不在目录 ⇒ 落回当次生效者（与 `llm_round_trip` 同一条规则）。
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    messages: Vec<ChatNode>,
    #[serde(default)]
    tools: Vec<CapabilityMeta>,
    #[serde(default)]
    root_id: String,
}

/// 一次模型往返的执行体。
struct ModelExecutor {
    active: Arc<dyn ModelProvider>,
    catalog: Vec<Arc<dyn ModelProvider>>,
}

#[async_trait::async_trait]
impl PortExecutor for ModelExecutor {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let request: RoundTripRequest = match serde_json::from_value(input.payload) {
            Ok(request) => request,
            Err(error) => {
                return PortOutcome::failed(
                    PortErrorClass::NonRetryable,
                    format!("模型请求载荷不可读：{error}"),
                )
            }
        };
        let provider = request
            .provider
            .as_deref()
            .and_then(|id| self.catalog.iter().find(|p| p.provider_id() == id))
            .unwrap_or(&self.active);
        let env = ExecEnv::new(exec.sink.clone(), exec.abort.clone());
        match provider
            // 债的预算与租约由环在派发前盖章；执行上下文（deadline / abort / 幂等键）
            // 原样交进来，执行体内不再另开一档校验（D-305 裁决一）。
            // internal-model-allow IM-001/gate: 端口执行体转发欠债行给的工具集（D-305 裁决一）。
            .round_trip(
                &request.system_prompt,
                &request.messages,
                &request.tools,
                &request.root_id,
                &env,
            )
            .await
        {
            Ok(output) => {
                let calls: Vec<serde_json::Value> = output
                    .tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "id": call.id,
                            "wire_id": call.wire_id,
                            "name": call.name,
                            "arguments": call.arguments,
                            "parse_error": call.parse_error,
                            "overflow_notice": call.overflow_notice,
                        })
                    })
                    .collect();
                // 一次往返的产物 → 事实草稿（键名是本插件的行契约，环不解析）。
                PortOutcome {
                    facts: vec![serde_json::json!({
                        "text": output.text,
                        "reasoning": output.reasoning,
                        "response_id": output.response_id,
                        "tool_calls": calls,
                        "finish": finish_str(&output.finish),
                        "usage": output.usage.as_ref().map(|u| serde_json::json!({
                            "input": u.input,
                            "output": u.output,
                        })),
                        "spend": output.spend.as_ref().map(|s| serde_json::json!({
                            "amount_nano": s.amount_nano,
                            "currency": s.currency,
                        })),
                        "response_text_child_id": output.response_text_child_id,
                        "reasoning_child_id": output.reasoning_child_id,
                    })],
                    error: None,
                }
            }
            Err(PluginError::Aborted) => PortOutcome {
                // 半截产出仍要留痕：中止是这一笔的状态，不是「没有发生过」。
                facts: vec![serde_json::json!({"status": "aborted"})],
                error: Some((PortErrorClass::NonRetryable, "aborted".to_string())),
            },
            Err(PluginError::RateLimited(why)) => {
                PortOutcome::failed(PortErrorClass::Retryable, why)
            }
            Err(error) => PortOutcome::failed(PortErrorClass::NonRetryable, error.to_string()),
        }
    }
}

/// 一次往返的产物键名归本插件的行契约；环不解析。
fn finish_str(finish: &ModelFinishReason) -> &str {
    match finish {
        ModelFinishReason::Stop => "stop",
        ModelFinishReason::Length => "length",
        ModelFinishReason::ToolCalls => "tool_calls",
        ModelFinishReason::ContentFilter => "content_filter",
        ModelFinishReason::Other(_) => "other",
    }
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
