//! 广播出口：把"状态收敛 + 帧投递"集中到一处。
//!
//! 三个方法都只做**投递**，不含业务判定：
//! - `broadcast_error_with_idle`：可恢复错误路径的唯一出口（复位 `is_working` →
//!   broadcast Error 事件 → broadcast Status idle）；
//! - `broadcast_frame`：向本会话全部前端订阅者投递一帧，顺带清掉已断开的通道；
//! - `broadcast_status`：忙 / 闲状态（**节点状态**而非流式内容）+ VDFS 变更通知。
//!
//! 可见性：`broadcast_error_with_idle` 被 `consume.rs` 调用，标 `pub(super)`；
//! `broadcast_frame` / `broadcast_status` 原本即 `pub`（根文件的 `WorkingGuard::drop`
//! 也在调用），保持不变。

use super::*;

impl SessionPlugin {
    /// 集中发送"业务错误 + 状态收敛"：复位 `is_working` → broadcast Error 事件 → broadcast Status idle
    ///
    /// **目的**：所有可恢复错误路径都必须保证 `is_working` 收敛到 `false`，
    /// 否则前端会一直显示"AI 处理中"且 30 分钟内无任何信号能清；
    /// 同时后端 `is_working` 不复位会导致后续 resume 请求被 `session_busy` 守卫静默拒绝。
    pub(super) async fn broadcast_error_with_idle(
        &self,
        state: &Arc<ActiveSessionState>,
        error: impl Into<String>,
    ) {
        let err = error.into();
        // 先复位 is_working，确保后续 resume 请求不会被 session_busy 守卫拒绝
        {
            let mut inner = state.inner.write().await;
            if inner.is_working {
                inner.is_working = false;
            }
        }
        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Error {
                error: err,
            })),
        )
        .await;
        self.broadcast_status(state, "idle").await;
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

    pub async fn broadcast_status(&self, state: &Arc<ActiveSessionState>, status: &str) {
        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Status {
                status: status.to_string()
            })),
        )
        .await;

        // 会话的忙/闲是**节点状态**，不是流式内容：以一次 VDFS 变更通知订阅方，
        // 由列表/详情按 path 重读 `vdfs/stat` 取最新 status（角标即时刷新，
        // 初始态仍由清单接口兜底）。
        let id = state.request_id_str();
        match status {
            "busy" | "idle" => {
                self.notify_change(&id, vdfs::VDFS_CHANGE_UPDATED);
            }
            _ => {}
        }
    }
}
