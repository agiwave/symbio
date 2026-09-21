//! BoundProvider —— core `ModelProvider` 纯 trait 的生产实现。
//!
//! 绑定两样东西：
//! - [`ModelProviderConfig`]：持久化配置 schema（serde 字段名冻结，用户配置兼容）；
//! - [`ModelProtocol`]：插件私有的协议适配钩子实现（`Arc<dyn ModelProtocol>`）。
//!
//! 完整单轮行为（`execute_turn` 五态机、`effective_context_tokens` 收敛）由本
//! 类型实现，`TurnOutput` → `PluginError` 的映射固定为：
//! - `Aborted` → `PluginError::Aborted`；
//! - `RetryWithoutContextId` → `plugin_warn!` + 同名错误（半截流由 chat_loop 重试分支以
//!   `NodeOp::Remove` 状态变更清除）；
//! - `Err(msg)` → `PluginError::InternalError`；
//! - `RateLimited(msg)` → `PluginError::RateLimited`；
//! - `Ok(resp)` → `parse_sse_stream`（`Err` → `PluginError::StreamError`）。
//!
//! 协议差异全部被 `ModelProtocol` 钩子吸收，本实现对所有协议通用。

use crate::plugin_error;
use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::turn::{execute_post_with_abort, parse_sse_stream, PostResult, TurnOutput};
use crate::symbio_core::{CapabilityMeta, ExecEnv, ModelProvider, PluginError};
use async_trait::async_trait;
use std::sync::Arc;

use super::model_providers::ModelProviderConfig;
use super::protocols::ModelProtocol;

/// 持久化配置与协议实现的运行期绑定体（model 插件对 core 契约的唯一生产实现）
pub struct BoundProvider {
    cfg: ModelProviderConfig,
    protocol: Arc<dyn ModelProtocol>,
}

impl BoundProvider {
    /// 绑定持久化配置与协议实现
    pub fn new(cfg: ModelProviderConfig, protocol: Arc<dyn ModelProtocol>) -> Self {
        Self { cfg, protocol }
    }
}

#[async_trait]
impl ModelProvider for BoundProvider {
    fn provider_id(&self) -> &str {
        &self.cfg.id
    }

    fn api_protocol(&self) -> &str {
        &self.cfg.api_protocol
    }

    fn rate_limit_ms(&self) -> u64 {
        self.cfg.rate_limit_ms
    }

    fn max_context_tokens(&self) -> u32 {
        self.cfg.max_context_tokens
    }

    /// 生效上下文上限：`min(用户设置, 服务上报)`。
    ///
    /// 服务端能主动上报上限时（本地模型常见）取两者较小值，避免请求超限；
    /// 否则（云端 API 普遍不暴露）直接使用用户设置。
    async fn effective_context_tokens(&self) -> u32 {
        match self.protocol.query_context_limit(&self.cfg).await {
            Some(limit) => self.cfg.max_context_tokens.min(limit),
            None => self.cfg.max_context_tokens,
        }
    }

    /// 执行完整单轮 LLM 请求：构造请求体 → 带中止的 POST → SSE 流解析。
    ///
    /// 实现对所有协议通用：协议差异已被 `ModelProtocol` 的钩子吸收，
    /// 故无需（也不可）按协议覆写。
    async fn execute_turn(
        &self,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
        root_id: &str,
        env: &ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        // 执行期环境（出口 + 中止）由调用方一次给全，协议层只管自己的流。
        let sink = env.sink();
        let abort = env.abort();
        // ①「LLM 请求发起」日志：每轮请求的起点打点，含模型、消息数、工具数。
        // 后续所有阶段日志（受理/首包/首条内容/结束）均以本条为锚点串联成完整轨迹。
        let turn_started = std::time::Instant::now();
        plugin_info!(
            "model",
            "[LLM] 请求发起 (model={}, msgs={}, tools={}, root={})",
            self.cfg.model,
            messages.len(),
            tools.len(),
            root_id
        );
        let body = self
            .protocol
            .prepare_request(&self.cfg, system_prompt, messages, tools);

        let response = match execute_post_with_abort(
            &self.protocol.get_api_url(&self.cfg),
            self.protocol.get_headers(&self.cfg),
            &body,
            abort,
        )
        .await
        {
            PostResult::Aborted => {
                plugin_info!(
                    "model",
                    "[LLM] 轮次中止 (耗时 {:?})",
                    turn_started.elapsed()
                );
                return Err(PluginError::Aborted);
            }
            PostResult::RetryWithoutContextId => {
                plugin_warn!(
                    "model",
                    "[LLM] Response context lost (400). Retrying turn without response_ids... (耗时 {:?})",
                    turn_started.elapsed()
                );
                // 半截流的清除不在本层：返回错误后由 chat_loop 的重试分支对被废弃的
                // Streaming 节点逐条发 `NodeOp::Remove`（状态变更，前端据此
                // 移除视图），不再依赖已废除的一次性 Abort 事件帧。
                return Err(PluginError::RetryWithoutContextId);
            }
            PostResult::Err(msg) => {
                plugin_error!(
                    "model",
                    "[LLM] 轮次失败：{msg} (耗时 {:?})",
                    turn_started.elapsed()
                );
                return Err(PluginError::InternalError(msg));
            }
            PostResult::RateLimited(msg) => {
                plugin_error!(
                    "model",
                    "[LLM] 轮次失败：限流 (耗时 {:?})",
                    turn_started.elapsed()
                );
                return Err(PluginError::RateLimited(msg));
            }
            PostResult::Ok(resp) => {
                plugin_info!(
                    "model",
                    "[LLM] 响应已受理，开始接收流 (耗时 {:?})",
                    turn_started.elapsed()
                );
                resp
            }
        };

        let protocol = Arc::clone(&self.protocol);
        match parse_sse_stream(response, root_id, sink, abort, move |line| {
            protocol.parse_response_line(line)
        })
        .await
        {
            Err(msg) => {
                plugin_error!(
                    "model",
                    "[LLM] 轮次异常结束：{msg} (总耗时 {:?})",
                    turn_started.elapsed()
                );
                Err(PluginError::StreamError(msg))
            }
            Ok(out) => {
                plugin_info!(
                    "model",
                    "[LLM] 轮次完成 (总耗时 {:?})",
                    turn_started.elapsed()
                );
                Ok(out)
            }
        }
    }
}
