//! BoundProvider —— core `ModelProvider` 纯 trait 的生产实现。
//!
//! 绑定两样东西：
//! - [`ModelProviderConfig`]：持久化配置 schema（serde 字段名冻结，用户配置兼容）；
//! - [`ModelProtocol`]：插件私有的协议适配钩子实现（`Arc<dyn ModelProtocol>`）。
//!
//! 完整单轮行为（`round_trip` 五态机、`effective_context_tokens` 收敛）由本
//! 类型实现，`TurnOutput` → `PluginError` 的映射固定为：
//! - `Aborted` → `PluginError::Aborted`；
//! - `RetryWithoutContextId` → `plugin_warn!` + 同名错误（半截流由 chat_loop 重试分支以
//!   `status = removed` 的删除帧清除）；
//! - `Err(msg)` → `PluginError::InternalError`；
//! - `RateLimited(msg)` → `PluginError::RateLimited`；
//! - `Ok(resp)` → `parse_sse_stream`（`Err(StreamFailure)` → `PluginError::StreamError`）。
//!
//! **两个重试轴**（互不覆盖，见 `round_trip`）：
//! 1. 请求级（`http::execute_post_with_abort`）：响应头之前——连接失败 / 瞬时 5xx；
//! 2. 流级（本文件）：**流已建立之后**被上游掐断（`UND_ERR_SOCKET` / 502 断流）——
//!    请求级重试此时早已 `Ok`，一次都不会触发，故必须在这里补。
//!
//! 协议差异全部被 `ModelProtocol` 钩子吸收，本实现对所有协议通用。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_core::chat_node::ChatNode;
use symbio_core::plugin_error;
use symbio_core::plugin_info;
use symbio_core::plugin_warn;
use symbio_core::TurnOutput;
use symbio_core::{CapabilityMeta, ExecEnv, ModelProvider, PluginError};

use super::http::{endpoint_label, execute_post_with_abort, PostResult};
use super::model_providers::ModelProviderConfig;
use super::protocols::{ModelProtocol, SseLineParser};
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
    async fn round_trip(
        &self,
        system_prompt: &str,
        messages: &[ChatNode],
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
        // 模型、消息数、工具数、根节点，加上实际端点与报文体量。**只打一条**：拆成
        // 相邻两条同名「请求发起」（一条在此、一条在 `execute_post_with_abort` 内）
        // 会每次请求刷两行、语义重复。
        let turn_started = std::time::Instant::now();
        let body = self
            .protocol
            .prepare_request(&self.cfg, system_prompt, messages, tools);
        // 请求体**只序列化一次**：这一份字节既用来记日志里的「体量」，也用来实际发送。
        //
        // 若为量字节数另调 `to_vec`、发送侧再用 `.json(body)`，同一份请求体要序列化两遍；
        // 而 `.json()` 位于**重试循环内**（首次 1 次 + 最多 `MAX_RETRIES` 次重试），最坏是
        // 6 次，必要的是 1 次——重试恰恰发生在网络 / 服务端已经不健康时，「这点开销可忽略」
        // 少算了这一项。
        //
        // 顺带：日志里的「体量」由此**必然等于**实际发出的字节数，而不是「两份各自序列化
        // 出来的东西理论上应该相同」。
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

        let mut response = match execute_post_with_abort(
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
                // 半截流的清除不在本层（见下）：这里没有可携带的半截产出。
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

        // ── 流中途断线：原样重发（本函数的第二个重试轴）──────────────────
        //
        // 请求级重试（`http::execute_post_with_abort`）覆盖的是**响应头之前**：拿不到
        // 连接、拿不到 2xx 才重发。但上游常见故障发生在**流已建立之后**——网关读到一半
        // 把 TCP 关掉（`UND_ERR_SOCKET` / 502 Upstream Stream Terminated），此时
        // `PostResult` 早已是 `Ok`，请求级重试一次都不会触发，`parse_sse_stream` 直接
        // `Err` ⇒ 整轮失败、会话停在错误态，用户只能手动重发。`c2a87a21` 号会话正是
        // 这个形态（5 个工具跑完后，后续请求被上游掐断）。
        //
        // 重发是安全的，因为副作用都在**流层**且可被覆盖：
        // - 已广播的正文 / 推理子节点 id 记在 `out` 里，跨尝试保留 ⇒ 重试时同一节点
        //   被完整快照覆盖，不新建第二条（`dispatch_and_track` 的首增量为整条快照）；
        // - 已广播的 ToolCall 节点 id 由 `reuse_node_ids` 播种进累积器，同理整条覆盖；
        // - 根 Turn 节点由 session 侧在轮次开头创建一次，不在这里产生；
        // ⇒ 重试不产生任何「重复卡片」。
        //
        // 只重发**瞬时**故障（`StreamFailure::transient`）：协议帧里的 `error` 事件是
        // 上游明确拒绝，原样重发只会再被拒，白等 N 次。
        const MAX_STREAM_RETRIES: u32 = 2;
        let mut stream_attempt: u32 = 0;
        let mut out = TurnOutput::default();
        let mut reuse_node_ids: Vec<String> = Vec::new();
        loop {
            match parse_sse_stream(
                response,
                root_id,
                sink,
                abort,
                parser,
                &reuse_node_ids,
                &mut out,
            )
            .await
            {
                Ok(()) => {
                    // 钱与 token 同一次测量、同一个边界：价格是本 Provider 的配置，
                    // 用量是 provider 这次实测的数（见 `ModelPricing::charge`）。
                    // 没有实测用量 ⇒ 没有可折算的金额（`charge` 返回 `None`）。
                    if let (Some(pricing), Some(usage)) = (&self.cfg.pricing, out.usage) {
                        out.spend = pricing.charge(usage);
                    }
                    return Ok(out);
                }
                Err(fail) if fail.transient && stream_attempt < MAX_STREAM_RETRIES => {
                    stream_attempt += 1;
                    plugin_warn!(
                        "model",
                        "[LLM] 流中途断线，第{stream_attempt}/{MAX_STREAM_RETRIES}次原样重发\
                         （已建 {} 个工具调用节点将复用 id）",
                        out.tool_calls.len()
                    );
                    // 本批尝试已广播的节点 id 交给下一批：重发时整条覆盖而不是新建。
                    reuse_node_ids = out
                        .tool_calls
                        .iter()
                        .filter_map(|tc| tc.id.clone())
                        .collect();
                    // 退避与请求级重试**同一条曲线**（`http::backoff_delay`）：断流多因上游
                    // 瞬时过载，立刻重打只会再断一次。等待同样响应中止信号。
                    let delay = super::http::backoff_delay(stream_attempt, None);
                    plugin_info!(
                        "model",
                        "[LLM] 流重发退避 {:?}（第{stream_attempt}次，与请求级同曲线）",
                        delay
                    );
                    super::http::sleep_with_abort(delay, abort).await;
                    if abort.is_aborted() {
                        // 半截流的清除不在本层（见 `PostResult::Aborted` 首次分支）。
                        return Err(PluginError::Aborted);
                    }
                    // 响应体已被上一批读掉（`bytes_stream` 消费），必须重新 POST。
                    // 重发请求不再递归重试：请求级语义（含它自己的 4 次重试）已覆盖。
                    match execute_post_with_abort(
                        &self.protocol.get_api_url(&self.cfg),
                        self.protocol.get_headers(&self.cfg),
                        &body_bytes,
                        abort,
                    )
                    .await
                    {
                        PostResult::Ok(r) => response = r,
                        // 重发连请求都没发出去：即时失败也没得重试了，按请求级措辞收场。
                        PostResult::Err(m) | PostResult::RateLimited(m) => {
                            plugin_error!(
                                "model",
                                "[LLM] 流重发失败（请求级）：{m} (总耗时 {:?})",
                                turn_started.elapsed()
                            );
                            return Err(PluginError::StreamError(m));
                        }
                        PostResult::Aborted => {
                            plugin_info!(
                                "model",
                                "[LLM] 轮次中止 (耗时 {:?})",
                                turn_started.elapsed()
                            );
                            // 半截流的清除不在本层（见 `PostResult::Aborted` 首次分支）。
                            return Err(PluginError::Aborted);
                        }
                        // 交给上层清 response_id 后重发整轮（与请求级首次尝试同口径）。
                        PostResult::RetryWithoutContextId => {
                            return Err(PluginError::RetryWithoutContextId);
                        }
                    }
                }
                Err(fail) => {
                    plugin_error!(
                        "model",
                        "[LLM] 轮次异常结束：{} (总耗时 {:?}, 流重发 {} 次)",
                        fail.message,
                        turn_started.elapsed(),
                        stream_attempt
                    );
                    return Err(PluginError::StreamError(fail.message));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "bound_provider.test.rs"]
mod tests;
