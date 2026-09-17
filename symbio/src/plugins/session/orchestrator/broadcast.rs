//! 广播出口：把"状态收敛 + 帧投递"集中到一处。
//!
//! 三个方法都只做**投递**，不含业务判定：
//! - `emit_session_state`：**会话运行态的唯一出口**——写运行态 → 发带节点视图的
//!   VDFS 变更（→ 前端）→ 发 `Status` 帧（→ 进程内消费者）；
//! - `broadcast_error_with_idle`：可恢复错误路径的唯一出口（收敛为"失败"结局
//!   → Error 帧）；
//! - `broadcast_frame`：向本会话全部前端订阅者投递一帧，顺带清掉已断开的通道。
//!
//! ## 为什么运行态要经 VDFS 变更而不是事件
//!
//! 见 `session/docs/node-state-streaming.md`：状态是节点的属性，变更携带**全量
//! 节点视图**，因此幂等、可交换、丢一次不影响正确性；而事件是增量的、有顺序的
//! （`busy` 与 `idle` 谁先到决定 UI 对错）。前端因此不再订阅 `kind = "session"`。
//!
//! `Status` 帧**仍然发送**：它是给**进程内**消费者的契约
//! （`agent/host/subagent.rs` 的转播任务以 `Status idle` 判定子会话结束），
//! 与前端显示无关。前端不订阅该频道，收到也不处理。
//!
//! 可见性：`broadcast_error_with_idle` 被 `consume.rs` 调用，标 `pub(super)`；
//! `broadcast_frame` / `emit_session_state` 原本即 `pub`（根文件的
//! `WorkingGuard::drop` 也在调用），保持不变。

use super::*;

/// 会话运行态的一次变化。
///
/// 用枚举而不是 `(status: &str, error: Option<String>)` 两个参数：后者允许
/// "`busy` 还带着上次的错误"这种非法组合存在，而它的表现是**静默的**——
/// 前端会同时显示"处理中"和"上次失败"。
pub enum SessionStateChange {
    /// 新一轮开始（上一轮的结局随之作废）
    Working,
    /// 一轮结束：正常 / 用户中止 / 以错误结束
    Finished {
        outcome: &'static str,
        error: Option<String>,
    },
}

impl SessionStateChange {
    /// 对应的线路状态词（`StreamEvent::Status` 的取值，进程内契约）
    fn status_word(&self) -> &'static str {
        match self {
            SessionStateChange::Working => "busy",
            SessionStateChange::Finished { .. } => "idle",
        }
    }
}

impl SessionPlugin {
    /// 集中发送"业务错误 + 状态收敛"：收敛为「失败」结局 → 广播 Error 帧。
    ///
    /// **目的**：所有可恢复错误路径都必须保证运行态收敛到"不在跑"，
    /// 否则前端会一直显示"AI 处理中"且 30 分钟内无任何信号能清；
    /// 同时后端 `is_working` 不复位会导致后续 resume 请求被 `session_busy` 守卫静默拒绝。
    ///
    /// 错误的**呈现**不再靠这条帧：它已随会话节点视图（`status = failed` +
    /// `attributes.error`）下发，`Error` 帧只承担进程内消费者（子会话转播）的语义。
    pub(super) async fn broadcast_error_with_idle(
        &self,
        state: &Arc<ActiveSessionState>,
        error: impl Into<String>,
    ) {
        let err = error.into();
        self.emit_session_state(
            state,
            SessionStateChange::Finished {
                outcome: OUTCOME_FAILED,
                error: Some(err.clone()),
            },
        )
        .await;
        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Error {
                error: err
            })),
        )
        .await;
    }

    /// 会话运行态变化的**唯一出口**。
    ///
    /// 三件事，顺序写死：
    /// 1. **写运行态**——节点视图的数据源（`list` / `stat` / 变更三处因此同源）；
    /// 2. **发 VDFS 变更**（带节点视图）——前端据此渲染，**零回读**；
    /// 3. **发 `Status` 帧**——进程内消费者（子会话转播）的结束判据。
    ///
    /// 漏掉第 2 步，UI 会永久停在旧状态（角标不转、停止按钮不出现/不消失），
    /// 且没有任何机制会纠正它——两条链路互不校验。
    pub async fn emit_session_state(
        &self,
        state: &Arc<ActiveSessionState>,
        change: SessionStateChange,
    ) {
        let id = state.request_id_str();
        {
            let mut inner = state.inner.write().await;
            match &change {
                SessionStateChange::Working => {
                    inner.is_working = true;
                    // 新一轮开始：上一轮的结局作废，否则失败角标会残留
                    inner.last_outcome = None;
                    inner.last_error = None;
                }
                SessionStateChange::Finished { outcome, error } => {
                    inner.is_working = false;
                    inner.last_outcome = Some((*outcome).to_string());
                    inner.last_error = error.clone();
                }
            }
        }

        // 前端通道：状态是节点属性，变更带全量节点视图 ⇒ 幂等、与顺序无关
        self.notify_session_state(&id).await;

        // 进程内通道：`Status` 帧仍是子会话转播的结束判据（前端不订阅本频道）
        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Status {
                status: change.status_word().to_string()
            })),
        )
        .await;
    }

    pub async fn broadcast_frame(&self, state: &Arc<ActiveSessionState>, frame: PluginFrame) {
        let mut inner = state.inner.write().await;
        let mut to_remove = Vec::new();
        for (idx, tx) in inner.frontends.iter().enumerate() {
            if tx.send(frame.clone()).await.is_err() {
                to_remove.push(idx);
            }
        }
        for idx in to_remove.into_iter().rev() {
            inner.frontends.remove(idx);
        }

        // 同时通过 EventBus 转发（供前端单连接订阅使用）
        if let PluginFrame::Data(data) = &frame {
            EventBus::try_publish("session", Some(&state.request_id_str()), data.clone());
        }
    }
}
