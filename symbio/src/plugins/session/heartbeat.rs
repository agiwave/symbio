//! 会话心跳任务调度器
//!
//! 后端常驻后台循环：周期性扫描所有会话，对启用了心跳任务且已空闲超过
//! `interval_seconds` 的会话（且当前未处于工作状态），自动发起一次提示词对话。
//!
//! 心跳任务配置存储于 `Session.metadata.heartbeat`，由前端"会话设置"写入。

use super::plugin::SessionPlugin;
use super::types::HeartbeatConfig;
use crate::symbio_core::now_ms;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::schemas::session::session_chat;
use crate::symbio_core::{PluginInvokeRequestExt, PluginSimpleRequest, SESSION_ID};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

/// 心跳调度器扫描间隔（秒）
const HEARTBEAT_TICK_SECS: u64 = 15;
/// 每次扫描最多触发的会话数（防止启动瞬间大量旧会话同时打爆模型服务）
const HEARTBEAT_MAX_PER_TICK: usize = 2;
/// 心跳间隔的确定性抖动上限（秒）：按会话 id 哈希错峰，避免同间隔会话同时触发
const HEARTBEAT_JITTER_SECS: u64 = 30;

/// 按会话 id 派生的确定性抖动（0..HEARTBEAT_JITTER_SECS 秒），用于错峰触发。
///
/// 采用确定性哈希而非随机：同一会话每次求得的抖动稳定，避免不同 tick 间反复抖动；
/// 同时让间隔相同的多个会话按 id 自然错开，缓解启动惊群。
/// 对早已超时的会话，抖动相对 `interval_seconds` 可忽略，仍会在首个 tick 触发。
fn heartbeat_phase_ms(id: &str) -> i64 {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    (hasher.finish() % HEARTBEAT_JITTER_SECS) as i64 * 1_000
}

/// 空闲基线：内存活动锚点与磁盘侧 `updated_at` 取较新者。
///
/// - 正常路径：回合内的消息落盘持续刷新 `updated_at`，其最终值 ≈ 活动真正结束
///   时刻，空闲时长因此从「会话无活动之后」起算；
/// - 异常路径：回合零写盘退出时 `updated_at` 停留在回合开始前，触发时写入的
///   内存锚点（较新）作为下限，防止逐 tick 热循环重触发。
fn idle_baseline(anchor: Option<i64>, updated_at: i64) -> i64 {
    anchor.map_or(updated_at, |a| a.max(updated_at))
}

impl SessionPlugin {
    /// 记录会话最近一次"有效活动"时间。
    ///
    /// 心跳调度器据此判断会话是否已空闲足够久：仅当用户/心跳消息被处理后才更新，
    /// 因此会话在两次活动之间一旦空闲达到间隔，即触发一次心跳。
    ///
    /// 联动语义：调度器以 `max(此锚点, 磁盘侧 updated_at)` 为空闲基线（见
    /// [`Self::run_heartbeat_loop`] 与 [`idle_baseline`]）——回合内的消息落盘会
    /// 持续刷新 `updated_at`，使空闲起点自然推进到活动真正结束（而非消息接收
    /// 时刻）；此锚点仅作为回合零写盘异常退出时的防热循环下限。
    pub(crate) async fn mark_activity(&self, session_id: &str) {
        let mut map = self.heartbeat_state.write().await;
        map.insert(session_id.to_string(), now_ms());
    }

    /// 心跳调度器主循环（在 [`SessionPlugin`] 构建时以 `tokio::spawn` 启动，常驻运行）。
    ///
    /// 每 [`HEARTBEAT_TICK_SECS`] 秒扫描一次：
    /// 1. 跳过未启用心跳或提示词为空的会话；
    /// 2. 正在工作的会话不启动心跳任务；
    /// 3. 会话空闲（无有效活动）达到 `interval_seconds` 后触发一次心跳；空闲基线
    ///    取 `max(内存活动锚点, 磁盘侧 updated_at)`，即从活动真正结束起算。
    pub(crate) async fn run_heartbeat_loop(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(Duration::from_secs(HEARTBEAT_TICK_SECS));
        // 跳过首tick（interval 首次 tick 立即返回），避免启动瞬间集中触发
        ticker.tick().await;

        crate::plugin_info!(
            "session",
            "[Heartbeat] 调度器已启动（扫描间隔 {}s）",
            HEARTBEAT_TICK_SECS
        );

        loop {
            ticker.tick().await;

            let sessions = match self.list_sessions().await {
                Ok(s) => s,
                Err(e) => {
                    crate::plugin_warn!("session", "[Heartbeat] list_sessions 失败: {}", e);
                    continue;
                }
            };

            let now = now_ms();
            // 每轮扫描最多触发有限个会话：避免启动时大量超时间隔的旧会话
            // 在同一 tick 内瞬间并发打爆模型服务（惊群），让其随 tick 自然错峰。
            let mut fired = 0usize;
            for s in sessions {
                if fired >= HEARTBEAT_MAX_PER_TICK {
                    break;
                }

                let hb = HeartbeatConfig::from_metadata(&s.metadata);
                if !hb.enabled || hb.prompt.trim().is_empty() {
                    continue;
                }

                // 正在工作的会话不启动心跳任务（防重入：心跳回合进行中绝不再次触发）
                let state = self.active_mgr.get_or_create(&s.id).await;
                if state.inner.read().await.is_working {
                    // 空闲基线无需在此维护：updated_at 随回合内每条消息落盘持续
                    // 刷新（含错误/panic 收敛时的失败持久化），回合最后一次写盘
                    // ≈ 活动真正结束，下方 max(锚点, updated_at) 基线自然推进。
                    continue;
                }

                // 空闲计时起点：内存活动锚点与磁盘侧 updated_at 取较新者。
                // - updated_at 随每条消息落盘刷新，回合最后一次写盘 ≈ 活动真正
                //   结束，保证「间隔」从会话无活动之后起算（而非从上次触发或
                //   消息接收时刻起算）；
                // - 触发时写入的内存锚点作为下限：回合零写盘异常退出（如磁盘
                //   故障导致失败持久化也失败）时，防止逐 tick 热循环重触发。
                let last_activity = idle_baseline(
                    self.heartbeat_state.read().await.get(&s.id).copied(),
                    s.updated_at,
                );
                // 按会话 id 派生确定性抖动，使同间隔的多个会话自然错峰，
                // 进一步缓解启动惊群（对早已超时的会话影响可忽略）。
                let interval_ms = (hb.interval_seconds as i64) * 1_000 + heartbeat_phase_ms(&s.id);

                if now - last_activity >= interval_ms {
                    self.clone().trigger_heartbeat(&s.id, &hb).await;
                    // 写入内存锚点作为防热循环下限：心跳回合内的消息落盘会把
                    // updated_at 推进到回合结束，下一次触发自然在「回合结束后
                    // 再空闲 interval」时到来；仅当回合零写盘退出时由此锚点兜底。
                    {
                        let mut map = self.heartbeat_state.write().await;
                        map.insert(s.id.clone(), now);
                    }
                    fired += 1;
                }
            }
        }
    }

    /// 立即为指定会话触发一次心跳任务。
    ///
    /// 构造一条用户消息（心跳提示词）并复用统一入口 [`SessionPlugin::handle_chat_send_oneoff`]
    /// 的发送链路。`include_history=false` 时本次发送不加载历史会话信息。
    pub(crate) async fn trigger_heartbeat(self: Arc<Self>, session_id: &str, hb: &HeartbeatConfig) {
        let now = now_ms();
        let user_msg = cm::ChatMessage {
            id: format!("hb_{}_{}", session_id, now),
            role: Some(cm::MessageRole::User),
            msg_type: Some(cm::MessageType::Text),
            content: Some(cm::MessageContent::Text(hb.prompt.clone())),
            status: Some(cm::MessageStatus::Completed),
            timestamp: Some(now),
            // 标记这是系统心跳任务自动发送的消息，便于前端区分展示
            meta: Some(serde_json::json!({ "heartbeat": true })),
            ..Default::default()
        };

        let req = session_chat::Request {
            session_id: Some(session_id.to_string()),
            // agent_id 留空：由 handle_chat_send_oneoff 回退到会话 metadata.agent_id
            agent_id: None,
            message: Some(user_msg),
            provider_id: None,
            include_history: Some(hb.include_history),
            // 心跳任务默认走 auto 模式（无人值守），避免后台触发遇到需交互工具时产卡阻塞。
            mode: Some("auto".to_string()),
            // risk_level 留空：由 orchestrator 回退到会话 metadata.risk_level（默认 medium）。
            // 心跳任务应尊重会话自身的风险等级设置，而非强行覆盖。
            risk_level: None,
            resume: None,
        };

        let ctx = PluginSimpleRequest::new(None, None);
        // 刻意**不设 `PATH`**：下面走的是直连方法调用（`self.handle_chat_send_oneoff`），
        // 不过路由，`PATH` 没有任何读者。这里曾写 `ctx.set(PATH, "chat/send")`——
        // 纯死赋值，而且值还是相对臂（相对臂只属于插件自己的 `match`，不该出现在调用侧）。
        // 编排侧的口径见 `orchestrator/entry.rs`：「chat_ctx 仅承载 payload（不再设置
        // 跨插件 PATH）」。地址规则见 `symbio_core::plugin::route` 模块文档。
        ctx.set(SESSION_ID, session_id.to_string());
        if let Err(e) = ctx.set_payload(req) {
            crate::plugin_error!("session", "[Heartbeat] 触发失败：无法设置 payload: {}", e);
            return;
        }

        crate::plugin_info!(
            "session",
            "[Heartbeat] 触发会话 {} 的心跳任务（include_history={}）",
            session_id,
            hb.include_history
        );

        // handle_chat_send_oneoff 内部会自行置 is_working 并路由到模型插件；
        // 即便没有前端连接，事件也会经由 EventBus 静默丢失，不影响后台执行。
        if let Err(e) = self.handle_chat_send_oneoff(Arc::new(ctx)).await {
            crate::plugin_error!(
                "session",
                "[Heartbeat] 触发失败：handle_chat_send_oneoff 返回错误: {}",
                e
            );
        }
    }
}

#[cfg(test)]
#[path = "heartbeat.test.rs"]
mod tests;
