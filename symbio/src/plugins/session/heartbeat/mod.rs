//! 会话心跳任务调度器
//!
//! 后端常驻后台循环：周期性扫描所有会话，对启用了心跳任务且已空闲超过
//! `interval_seconds` 的会话（且当前未处于工作状态），自动发起一次提示词对话。
//!
//! 心跳任务配置存储于 `Session.metadata.heartbeat`，由前端"会话设置"写入。

use super::active::PreemptPending;
use super::plugin::SessionPlugin;
use crate::symbio_core::chat_message as cm;
use crate::symbio_core::clock_now_ms;
use crate::symbio_core::session_chat;
use crate::symbio_core::{
    AutonomousInitiator, ConationCandidate, ConationPolicy, EventWalStore, IntentGate,
    PluginInvokeRequestExt, PluginSimpleRequest, Seq, Store, EVENT_TASK_OPENED, SESSION_ID,
};
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
            ticker.tick().await;

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
                    let fired_now = self
                        .clone()
                        .trigger_heartbeat(&s.id, &hb, now - last_activity)
                        .await;
                    if fired_now {
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

    /// 立即为指定会话触发一次心跳任务。
    ///
    /// 构造一条用户消息（心跳提示词）并复用统一入口 [`SessionPlugin::handle_chat_send_oneoff`]
    /// 的发送链路。`include_history=false` 时本次发送不加载历史会话信息。
    ///
    /// 返回 `true` = 这次触发的**事实已落格**（调用方据此推进空闲锚点）；
    /// `false` = 这一趟没触发、一格也没写（原因在 [`Self::record_autonomous_trigger`]）。
    ///
    /// ## 先落事实，再开轮
    ///
    /// 第一步是把这次触发写成事实（[`Self::record_autonomous_trigger`]）：**事实没落成
    /// 就不发这一轮**。「触发器产出事件，不是旁路」（[roadmap/S12 §2](../../../../../docs/plan/roadmap/S12-自主层与长期目标.md)）
    /// 的可执行形式就是这句——否则自主行为从头到尾只有那条伪装成用户消息的提示词，
    /// 在事实源里连"系统自己发起过"都查不到。
    pub(crate) async fn trigger_heartbeat(
        self: Arc<Self>,
        session_id: &str,
        hb: &HeartbeatConfig,
        idle_ms: i64,
    ) -> bool {
        if !self
            .record_autonomous_trigger(session_id, idle_ms, &hb.prompt)
            .await
        {
            return false;
        }
        let now = clock_now_ms();
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
            // 触发事实已经落格了：这一趟按"触发过"算（`true`），否则下个 tick 会
            // 再写一遍——而 `system.triggered` 是**事实**，重复记一次就是两次触发。
            return true;
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
        true
    }

    /// 把一次定时触发落成事实（S9 第 21 步，[roadmap/S12](../../../../../docs/plan/roadmap/S12-自主层与长期目标.md)）。
    ///
    /// 返回 `true` = 触发事实已入格；`false` = 这一趟不触发（原因已记日志）。
    ///
    /// ## 三种不触发，各说各的
    ///
    /// 1. **有写方在写**——事实源是同一会话的唯一写入口（`WalStore::open` 各自
    ///    replay，两个写方并存会重复 `head` ⇒ 盘上出现重复 seq、`seq_monotonic`
    ///    直接变红）。见下「谁有资格写」；
    /// 2. **没有事实源 / 事实源为空**——溯源锚是"上一格的 seq"，一格都没有就无处
    ///    可指。I2 要求触发事件带 `produced_by`（S12 §5 第 3 行：自主发起的工作
    ///    必须可追溯），**宁可不触发也不写一条来路不明的触发**；新会话收到第一条
    ///    消息后事实源自然就有了，下一趟就成；
    /// 3. **开档失败**——存储层问题，记错误。
    ///
    /// ## 谁有资格写
    ///
    /// 会话事实源只有两个写窗口，本函数把两条互斥条件都检查掉：
    ///
    /// - 收束期写方 `record_to_wal` 跑在 `is_working` 复位**之前** ⇒ 要求
    ///   `is_working == false`；
    /// - 收件箱的空闲写方（挂起落格 / 恢复落格）**只在 `preempt != None` 时**才
    ///   落格 ⇒ 要求 `preempt == PreemptPending::None`。
    ///
    /// 两条同时成立时没有第二个写方能开档。残余窗口是"检查之后一毫秒内跑完一整轮
    /// 插话"——那是既有设计已接受的一类风险（同款说明见
    /// `transcript/inbox.rs::append_preemption`），本函数不放大它：检查紧跟开档、
    /// 格子逐条落、失败逐条记日志。
    async fn record_autonomous_trigger(&self, session_id: &str, idle_ms: i64, goal: &str) -> bool {
        {
            let sessions = self.active_mgr.sessions.read().await;
            if let Some(state) = sessions.get(session_id) {
                let inner = state.inner.read().await;
                if inner.is_working || inner.preempt != PreemptPending::None {
                    crate::plugin_info!(
                        "session",
                        "[Heartbeat] 会话 {} 有写方在写，本次触发不落格（下个 tick 重试）",
                        session_id
                    );
                    return false;
                }
            }
        }

        // 溯源锚 = 上一格的 seq：触发是"从当时的状态出发"的，指过去的事件而不是
        // 指自己。`head == 0` 既包含"事实源不存在"也包含"空文件"——两种都没有上一格。
        // 顺带挡掉目录不存在的情形：再往下就会 `append`（写方入口对缺父目录是 panic）。
        let path = super::paths::session_dir(&self.storage_dir(), session_id)
            .join(super::paths::V2_WAL_FILE);
        let store = match EventWalStore::open(&path) {
            Ok(store) => store,
            Err(e) => {
                crate::plugin_error!(
                    "session",
                    "[Heartbeat] 会话 {} 事实源开档失败：{}",
                    session_id,
                    e
                );
                return false;
            }
        };
        let head = store.head().value();
        if head == 0 {
            crate::plugin_info!(
                "session",
                "[Heartbeat] 会话 {} 还没有事实源，本次不触发（无可指的溯源锚）",
                session_id
            );
            return false;
        }
        let anchor = head - 1;

        let initiator = AutonomousInitiator;
        let triggered_seq = match store.append(initiator.trigger(anchor)) {
            Ok(seq) => seq.value(),
            Err(e) => {
                crate::plugin_error!(
                    "session",
                    "[Heartbeat] 触发事实未入格（会话 {}）：{:?}",
                    session_id,
                    e
                );
                return false;
            }
        };
        // 自检紧跟触发：触发是主事实，自检是这次触发的观测附录（`system × progressed`）。
        if let Err(e) = store.append(initiator.health_event(triggered_seq, idle_ms)) {
            crate::plugin_warn!(
                "session",
                "[Heartbeat] 健康自检未入格（会话 {}）：{:?}",
                session_id,
                e
            );
        }

        // 「欲」先入格（E1：欲是数据），闸门再决定它要不要升格成任务。
        let conation_seq = match store.append(initiator.express_intent(goal, triggered_seq)) {
            Ok(seq) => seq.value(),
            Err(e) => {
                crate::plugin_warn!(
                    "session",
                    "[Heartbeat] 意图未入格（会话 {}）：{:?}",
                    session_id,
                    e
                );
                // 触发已经落格 ⇒ 这一轮照常开跑（意图没写成不该连累触发）。
                return true;
            }
        };
        // 回读刚落格的那一条：候选的唯一构造路径要读 `seq`，而本地副本还是 pending。
        let source = store.range(Seq::new(conation_seq)).into_iter().next();
        let Some(source) = source else {
            crate::plugin_warn!(
                "session",
                "[Heartbeat] 意图回读不到（会话 {}，seq {}）",
                session_id,
                conation_seq
            );
            return true;
        };
        let Some(mut candidate) = ConationCandidate::from_event(&source) else {
            crate::plugin_warn!(
                "session",
                "[Heartbeat] 意图构造不出候选（会话 {}）",
                session_id
            );
            return true;
        };

        let policy = {
            let cfg = self.config.read().await;
            ConationPolicy {
                enabled: cfg.conation_enabled,
                ..ConationPolicy::default()
            }
        };
        match IntentGate::approve(&mut candidate, &policy, &IntentGate::issue_warrant()) {
            Ok(approved) => {
                // E2 的**动作点**：`task.opened` 只从盖过章的候选长出来。闸门是发牌口
                //（`issue_warrant` 是唯一一道），这一行验的是牌本身——少了它，「闸门与
                // 自主层同批成对」就退化成一句约定。
                if candidate.is_approved() && !long_goal_declared(&store, &approved.goal) {
                    let task_id = format!("hb-{triggered_seq}");
                    let opened =
                        initiator.open_long_goal(&task_id, &approved.goal, approved.from_seq);
                    if let Err(e) = store.append(opened) {
                        crate::plugin_error!(
                            "session",
                            "[Heartbeat] 长目标未入格（会话 {}）：{:?}",
                            session_id,
                            e
                        );
                    }
                }
            }
            Err(reason) => {
                crate::plugin_info!(
                    "session",
                    "[Heartbeat] 意图闸门拒绝（会话 {}）：{}",
                    session_id,
                    reason
                );
            }
        }
        true
    }
}

/// 事实源里有没有**已经声明过的**同一个长目标。
///
/// 长目标只声明一次：心跳按 `interval_seconds` 周期性表达同一条「欲」，每次都开格
/// 会让就绪集无界增长——而 `readyset` 是模型唯一的调度候选来源，那就等于用自己
/// 刷爆自己的提示词。重新发起是 S03 返工机制（`{id}-r{n}`）的事，不由心跳每 tick
/// 重开；「欲」本身照样每 tick 入格（它是流，不是状态）。
fn long_goal_declared(store: &EventWalStore, goal: &str) -> bool {
    store.range(Seq::new(0)).iter().any(|e| {
        e.kind == EVENT_TASK_OPENED && e.payload.get("goal").and_then(|v| v.as_str()) == Some(goal)
    })
}

#[cfg(test)]
mod tests;
