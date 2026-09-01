use super::types::*;
use crate::symbio_core::{InvokeRequest, InvokeResponse, Plugin, PluginChannel, PluginPayload};
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::sync::Arc;

mod anthropic_messages;
mod gemini_api;
mod openai_chat;
mod openai_responses;

/// 标准协议事件 - 用于将不同提供商的流解析为统一格式
#[derive(Debug, Clone)]
pub enum ProtocolEvent {
    /// 文本内容增量
    ContentDelta(String),
    /// 思考/推理过程增量
    ReasoningDelta(String),
    /// 工具调用增量 (index, id, name, arguments_delta)
    ToolCallDelta(usize, Option<String>, Option<String>, Option<String>),
    /// 响应 ID (用于 OpenAI Responses API)
    ResponseId(String),
    /// 错误信息
    Error(String),
}

/// MODEL 协议特质 - 抽象不同模型提供商的通信细节
#[async_trait]
pub trait ModelProtocol: Send + Sync {
    /// 获取 API 请求端点 URL
    fn get_api_url(&self, config: &ModelConfig) -> String;

    /// 获取 HTTP 请求头
    fn get_headers(&self, config: &ModelConfig) -> HeaderMap;

    /// 构造请求体 JSON
    fn prepare_request(
        &self,
        config: &ModelConfig,
        system_prompt: &str,
        messages: &[crate::symbio_core::schemas::session::chat_message::ChatMessage],
        tools: &[CapabilityMeta],
    ) -> Value;

    /// 获取用于验证配置的 minimal valid input
    fn get_validation_input(&self) -> Value {
        serde_json::json!({ "ping": true })
    }

    /// 解析流的一行内容（处理 SSE 协议）
    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent>;

    /// 处理流式聊天请求
    async fn handle_chat_stream(
        &self,
        config: &ModelConfig,
        parent: &Option<Arc<dyn Plugin>>,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload>;
}

/// 协议工厂 - 通过通用对象创建机制按 id 构造
///
/// 各协议实现各自在自己的文件里提供 `build` 函数并通过
/// `submit_object_creator!` 自动注册；调用方使用
/// `create_object::<dyn ModelProtocol>(id, ctx)` 取得实例。
///
/// 已注册 id（参见各协议文件）：
/// - `openai_responses`（别名 `responses`）— openai_responses.rs
/// - `openai_chat`（别名 `chat`）— openai_chat.rs
/// - `anthropic_messages`（别名 `anthropic`）— anthropic_messages.rs
/// - `gemini_api`（别名 `gemini`）— gemini_api.rs
///
/// 将 `ModelConfig.api_protocol` 别名解析为已注册的协议 id
pub fn resolve_protocol_id(name: &str) -> &'static str {
    match name {
        "openai_responses" | "responses" => "openai_responses",
        "openai_chat" | "chat" => "openai_chat",
        "anthropic_messages" | "anthropic" => "anthropic_messages",
        "gemini_api" | "gemini" => "gemini_api",
        // 默认兜底：未识别的协议回退到 openai_chat
        _ => "openai_chat",
    }
}

// 协议助手：通用流分发器

pub async fn spawn_orchestrator(
    protocol: Box<dyn ModelProtocol>,
    config: &ModelConfig,
    parent: &Option<Arc<dyn Plugin>>,
    ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let (host_chan, plugin_chan) = PluginChannel::pair(4096);

    let orchestrator =
        super::context::ChatOrchestrator::new(config.clone(), parent.clone(), protocol);
    let ctx_clone = ctx.fork();
    let error_tx = plugin_chan.tx.clone();

    // 关键：保留 host_chan.tx 的一个克隆在 orchestrator 任务中，
    // 直到 chat_loop 完全结束才释放。
    //
    // 原因：`plugin_chan.rx`（即 chat_loop 用来监听中止/审批信号的通道）唯一的
    // 发送者就是 `host_chan.tx`，而 `host_chan.tx` 是在 `handle_chat_session_internal`
    // 里 spawn 出 `backward_task` 时才会被持有。如果 chat_loop 比 backward_task
    // 更早进入 `wait_for_abort_signal`，则 plugin_chan.rx 当前没有任何 sender，
    // mpsc::Receiver::recv 会立即返回 `None`，导致 `wait_for_abort_signal` 误判
    // 为“通道关闭 → Aborted”，chat 在真正发起 HTTP 请求之前就被中断。
    //
    // 通过在本任务中保留一个 `host_chan.tx` 克隆（直到 chat_loop 返回后才离开
    // 作用域被 drop），可以保证 `plugin_chan.rx` 在 chat_loop 运行期间始终至少有
    // 一个 sender，`recv()` 不会再因为“暂时没有 sender”而提前返回。
    let host_tx_keepalive = host_chan.tx.clone();

    tokio::spawn(async move {
        // 持有 keepalive，直到 chat_loop 结束
        let _host_tx_keepalive = host_tx_keepalive;

        // 使用 catch_unwind 捕获 panic，确保任何错误都能正确返回给前端
        let result = tokio::task::spawn(async move {
            super::chat_loop::run_chat_loop(&orchestrator, ctx_clone, plugin_chan).await
        })
        .await;

        match result {
            Ok(Ok(_)) => {
                // 正常完成
            }
            Ok(Err(e)) => {
                // 捕获到 Result::Err
                let _ = error_tx
                    .send(crate::symbio_core::PluginFrame::Error(
                        e.to_string(),
                        Some(serde_json::json!({"code": e.code()})),
                    ))
                    .await;
            }
            Err(e) => {
                // 捕获到 task join error（包括 panic）
                let panic_msg = if e.is_panic() {
                    "服务器内部发生未预期的错误".to_string()
                } else {
                    format!("任务执行失败: {}", e)
                };

                crate::plugin_error!("model", "Chat loop error: {}", panic_msg);

                let _ = error_tx
                    .send(crate::symbio_core::PluginFrame::Error(
                        format!("服务器内部错误: {}", panic_msg),
                        Some(serde_json::json!({"code": "INTERNAL_ERROR"})),
                    ))
                    .await;
            }
        }
        // chat_loop 结束、backward_task 也即将被释放，此时让 keepalive 离开作用域
    });

    Ok(PluginPayload::Session(host_chan))
}
