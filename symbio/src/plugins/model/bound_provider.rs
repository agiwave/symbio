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
//!   `status = removed` 的删除帧清除）；
//! - `Err(msg)` → `PluginError::InternalError`；
//! - `RateLimited(msg)` → `PluginError::RateLimited`；
//! - `Ok(resp)` → `parse_sse_stream`（`Err` → `PluginError::StreamError`）。
//!
//! 协议差异全部被 `ModelProtocol` 钩子吸收，本实现对所有协议通用。

use crate::plugin_error;
use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::TurnOutput;
use crate::symbio_core::{CapabilityMeta, ExecEnv, ModelProvider, PluginError, SseLineParser};
use async_trait::async_trait;
use std::sync::Arc;

use super::http::{endpoint_label, execute_post_with_abort, PostResult};
use super::model_providers::ModelProviderConfig;
use super::protocols::ModelProtocol;
use super::stream::parse_sse_stream;

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
        // ①「LLM 请求发起」日志：**每轮请求唯一的起点锚点**。后续所有阶段日志
        // （响应头 / 首字节 / 首条内容 / 流结束）都以它为准串联成完整轨迹。
        //
        // 位置在**请求体构建之后**：这样一条日志即可同时承载「规模」与「出口」——
        // 模型、消息数、工具数、根节点，加上实际端点与报文体量。此前这些信息被拆成
        // 相邻两条同名「请求发起」（一条在此、一条在 `execute_post_with_abort` 内），
        // 每次请求刷两行、语义重复，故合并为这一条。
        let turn_started = std::time::Instant::now();
        let body = self
            .protocol
            .prepare_request(&self.cfg, system_prompt, messages, tools);
        // 请求体**只序列化一次**：这一份字节既用来记日志里的「体量」，也用来实际发送。
        //
        // 此前这里是 `to_vec(&body).map(|v| v.len())`——**只为了量出字节数**，序列化出的
        // 那份立即丢弃；而 `execute_post_with_abort` 里的 `.json(body)` 又要再序列化一遍。
        // 更糟的是 `.json()` 位于**重试循环内**（首次 1 次 + 最多 `MAX_RETRIES` 次重试），
        // 所以最坏是同一份请求体被序列化 **6 次**，而必要的是 1 次。原注释
        // 「这点开销可忽略」少算了重试这一项，且重试恰恰发生在网络/服务端已经不健康时。
        //
        // 顺带：日志里的「体量」现在**必然等于**实际发出的字节数——此前它只是
        // 「两份各自序列化出来的东西理论上应该相同」。
        let body_bytes = match serde_json::to_vec(&body) {
            Ok(v) => v,
            Err(e) => {
                plugin_error!("model", "[LLM] 请求体序列化失败：{}", e);
                return Err(PluginError::InternalError(format!("请求体序列化失败: {e}")));
            }
        };
        plugin_info!(
            "model",
            "[LLM] 请求发起 {} (model={}, msgs={}, tools={}, root={}, body={} bytes)",
            endpoint_label(&self.protocol.get_api_url(&self.cfg)),
            self.cfg.model,
            messages.len(),
            tools.len(),
            root_id,
            body_bytes.len()
        );

        let response = match execute_post_with_abort(
            &self.protocol.get_api_url(&self.cfg),
            self.protocol.get_headers(&self.cfg),
            &body_bytes,
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
                // Streaming 节点逐条发**删除帧**（`status = removed` 的状态迁移，前端
                // 据此移除视图），不再依赖已废除的一次性 Abort 事件帧。
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

        // 行解析契约直接由协议实例提供（`ModelProtocol: SseLineParser`）：
        // 完整行与「未结束行」的增量提取都在协议层，core 不认识任何字段名。
        let parser: &dyn SseLineParser = self.protocol.as_ref();
        match parse_sse_stream(response, root_id, sink, abort, parser).await {
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
