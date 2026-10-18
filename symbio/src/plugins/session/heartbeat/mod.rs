//! 会话心跳任务调度器
//!
//! 后端常驻后台循环：周期性扫描所有会话，对启用了心跳任务且已空闲超过
//! `interval_seconds` 的会话（且当前未处于工作状态），自动发起一次提示词对话。
//!
//! 心跳任务配置存储于 `Session.metadata.heartbeat`，由前端"会话设置"写入。
//!
//! ## 本模块只管「什么时候开一轮」
//!
//! 「这一轮是系统自己发起的」要先在事实源里落成事实、运行参数要按会话自己的数据推导，
//! 那是自主触发侧的写窗口纪律，住在 [`SessionPlugin::request_autonomous_round`]
//! （`transcript/autonomous.rs`；归属裁决见
//! [ADR-076](../../../../../docs/010-adr/076.md "ADR-076")）。本模块因此不认识
//! 事实源、也不决定 `mode` / `include_history`。

use super::plugin::SessionPlugin;
use crate::symbio_core::chat_node as cm;
use crate::symbio_core::clock_now_ms;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

// ==================== 心跳配置（`Session.metadata.heartbeat`） ====================

/// 会话心跳任务配置
///
/// 存储于 `Session.metadata.heartbeat`，由前端"会话设置"写入。
/// 后端 [`crate::plugins::session::plugin::SessionPlugin`] 的后台调度器据此在会话空闲
/// 指定时间后自动发起一次提示词对话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// 是否启用心跳任务
    #[serde(default)]
    pub enabled: bool,
    /// 启动间隔（秒）：会话空闲达到该时长后触发一次心跳
    #[serde(default = "default_heartbeat_interval")]
    pub interval_seconds: u64,
    /// 心跳任务提示词（每次触发时作为一条用户消息发送给模型）
    #[serde(default)]
    pub prompt: String,
    /// 启动心跳时是否携带历史会话信息
    /// - `true`（默认）：心跳消息作为普通对话追加，模型能看到历史
    /// - `false`：本次发送不加载任何历史（"无上下文"心跳）
    #[serde(default = "default_heartbeat_include_history")]
    pub include_history: bool,
}

fn default_heartbeat_interval() -> u64 {
    300
}

fn default_heartbeat_include_history() -> bool {
    true
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: default_heartbeat_interval(),
            prompt: String::new(),
            include_history: default_heartbeat_include_history(),
        }
    }
}

impl HeartbeatConfig {
    /// 从会话 metadata 解析心跳配置。字段缺失时返回默认（未启用）配置。
    pub fn from_metadata(metadata: &serde_json::Value) -> Self {
        let Some(obj) = metadata.get("heartbeat").and_then(|v| v.as_object()) else {
            return Self::default();
        };
        let enabled = obj
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let interval_seconds = obj
            .get("interval_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(default_heartbeat_interval);
        let prompt = obj
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let include_history = obj
            .get("include_history")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(default_heartbeat_include_history);
        Self {
            enabled,
            interval_seconds,
            prompt,
            include_history,
        }
    }
}

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
        map.insert(session_id.to_string(), clock_now_ms());
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
            // 停机（见 `SessionPlugin::stop_gate`）：每个 tick 之前重新挂一次唤醒，
            // 因此 `stop` 之后本循环最多再走**半圈**（扫描已开始则走完这一轮扫描，
            // 但不再触发新一轮——`fire` 的判定在扫描体内，退出判据在它之前）。
            let (wake, stopping) = self.stop_gate();
            if stopping {
                break;
            }
            tokio::select! {
                _ = ticker.tick() => {}
                _ = wake => break,
            }

            let sessions = match self.list_sessions().await {
                Ok(s) => s,
                Err(e) => {
                    crate::plugin_warn!("session", "[Heartbeat] list_sessions 失败: {}", e);
                    continue;
                }
            };

            let now = clock_now_ms();
            // 每轮扫描最多触发有限个会话：避免启动时大量超时间隔的旧会话
            // 在同一 tick 内瞬间并发打爆模型服务（惊群），让其随 tick 自然错峰。
            let mut fired = 0usize;
            for s in sessions {
                if fired >= HEARTBEAT_MAX_PER_TICK {
                    break;
                }
                // 停机位可能在扫描途中才置上：此后**一条也不触发**。已触发出去的
                // 回合不归这里管——它由会话自己收尾（停机砍的是"起新回合"这件事）。
                if self.is_stopping() {
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
                    // 只有**触发事实落了格**才写内存锚点：这一趟没触发（无事实源 /
                    // 有写方在写 / 开档失败）时留着原锚点，下个 tick 重试——那几种
                    // 都是暂时的，按整段 `interval_seconds` 推迟等于把一次本可成功的
                    // 触发白白睡掉。
                    //
                    // 本模块只回答「什么时候该开一轮」。**开轮的资格**（先把这次触发
                    // 落成事实，落成了才入队）与**运行参数**（`mode` / `include_history`
                    // 按会话自己的数据推导）住在自主触发侧
                    // （[`SessionPlugin::request_autonomous_round`]，归属见
                    // [ADR-076 裁决一 #5/#6](../../../../../docs/010-adr/076.md "ADR-076")）。
                    let fired_now = self
                        .request_autonomous_round(
                            &s.id,
                            &s.metadata,
                            heartbeat_message(&s.id, &hb),
                            now - last_activity,
                        )
                        .await
                        .is_some();
                    if fired_now {
                        crate::plugin_info!(
                            "session",
                            "[Heartbeat] 已触发会话 {} 的心跳任务",
                            s.id
                        );
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
    }
}

/// 心跳这一轮要发出去的用户消息：配置里的提示词 + 一条 `meta.heartbeat` 标记。
///
/// 消息 id 带触发时刻的毫秒（`hb_<sid>_<毫秒>`），同一会话的两次心跳消息因此天然不同 id。
/// 「这条是系统自动发的」只用于前端区分展示；这次触发**是不是自主的**由事实源里的那四格
/// 说话（见 [`SessionPlugin::request_autonomous_round`]），不靠消息上的这个布尔值。
fn heartbeat_message(session_id: &str, hb: &HeartbeatConfig) -> cm::ChatNode {
    let now = clock_now_ms();
    cm::ChatNode {
        id: format!("hb_{}_{}", session_id, now),
        role: Some(cm::NodeRole::User),
        msg_type: Some(cm::NodeKind::Body),
        content: Some(cm::MessageContent::Text(hb.prompt.clone())),
        status: Some(cm::NodeStatus::Completed),
        timestamp: Some(now),
        meta: Some(serde_json::json!({ "heartbeat": true })),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;
