//! 单轮消息处理
//!
//! 职责：
//! - 发送 LLM 请求
//! - 解析 SSE 响应
//! - 执行工具调用
//! - 完成轮次

use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::schemas::session::session_chat_response;
use crate::symbio_core::{CapabilityMeta, PluginChannel, PluginError, PluginFrame};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::context::{ChatOrchestrator, TurnOutput};
use super::protocol::{execute_post_with_abort, parse_sse_stream, PostResult};
use super::tool_call::ToolCallInfo;
use crate::symbio_core::schemas::session::chat_message::ChatMessage;

pub struct TurnProcessor<'a> {
    orchestrator: &'a ChatOrchestrator,
}

impl<'a> TurnProcessor<'a> {
    pub fn new(orchestrator: &'a ChatOrchestrator) -> Self {
        Self { orchestrator }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_request(
        &self,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
        root_id: &str,
        channel: &mut PluginChannel,
        abort_flag: &Arc<AtomicBool>,
        // previous_response_id: Option<&str>,
    ) -> Result<TurnOutput, PluginError> {
        let turn_config = self.orchestrator.config.clone();
        let body = self.orchestrator.protocol.prepare_request(
            &turn_config,
            system_prompt,
            messages,
            tools,
        );

        plugin_info!(
            "model",
            "[DIAG] TurnProcessor::send_request entered, url={}, msg_count={}, tool_count={}",
            self.orchestrator.protocol.get_api_url(&turn_config),
            messages.len(),
            tools.len()
        );

        let response = match execute_post_with_abort(
            &self.orchestrator.protocol.get_api_url(&turn_config),
            self.orchestrator.protocol.get_headers(&turn_config),
            &body,
            channel,
            abort_flag,
        )
        .await
        {
            PostResult::Aborted => {
                plugin_warn!(
                    "model",
                    "[DIAG] TurnProcessor::send_request: PostResult::Aborted"
                );
                return Err(PluginError::Aborted);
            }
            PostResult::RetryWithoutContextId => {
                plugin_warn!(
                    "model",
                    "Response context lost (400). Retrying turn without response_ids..."
                );
                emit_abort(channel).await;
                return Err(PluginError::RetryWithoutContextId);
            }
            PostResult::Err(msg) => {
                plugin_warn!(
                    "model",
                    "[DIAG] TurnProcessor::send_request: PostResult::Err({})",
                    msg
                );
                return Err(PluginError::InternalError(msg));
            }
            PostResult::RateLimited(msg) => {
                plugin_warn!(
                    "model",
                    "[DIAG] TurnProcessor::send_request: PostResult::RateLimited({})",
                    msg
                );
                return Err(PluginError::RateLimited(msg));
            }
            PostResult::Ok(resp) => {
                plugin_info!(
                    "model",
                    "[DIAG] TurnProcessor::send_request: PostResult::Ok, status={}",
                    resp.status()
                );
                resp
            }
        };

        match parse_sse_stream(
            response,
            root_id,
            channel,
            abort_flag,
            self.orchestrator.protocol.as_ref(),
        )
        .await
        {
            Err(msg) => {
                plugin_warn!(
                    "model",
                    "[DIAG] TurnProcessor::send_request: parse_sse_stream Err({})",
                    msg
                );
                Err(PluginError::StreamError(msg))
            }
            Ok(out) => {
                plugin_info!(
                    "model",
                    "[DIAG] TurnProcessor::send_request: parse_sse_stream Ok, text_len={}, reasoning_len={}, tool_calls={}",
                    out.text.len(),
                    out.reasoning.len(),
                    out.tool_accumulator.get_completed().len()
                );
                Ok(out)
            }
        }
    }

    pub async fn finalize(
        &self,
        root_id: &str,
        out: &TurnOutput,
        tools: &[ToolCallInfo],
        channel: &PluginChannel,
    ) {
        self.orchestrator
            .finalize_assistant_turn(root_id, out, tools, channel)
            .await;
    }
}

async fn emit_abort(channel: &PluginChannel) {
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Abort {}).unwrap_or_default(),
        ))
        .await;
}
