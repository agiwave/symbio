//! 执行期事件出口的生产实现（[`TranscriptWriter`] → 转写唯一写入点）。
//!
//! ## 为什么在编排层
//!
//! 本文件取代的是**历史上消费循环里的那段分派**：
//!
//! ```text
//! // 收口前（consume.rs）
//! match serde_json::from_value::<NodeOp>(data.clone()) {
//!     Ok(NodeOp::Warn { warning }) => emit_session_state(...),   // 会话级状态
//!     Ok(op)                        => transcript.apply(op),     // 转写面
//!     Err(e)                        => plugin_warn!("无法解析的数据帧"),
//! }
//! ```
//!
//! 分派规则（`Warn` 归会话节点、其余归转写）属于**编排语义**，因此实现落在这里，
//! 而不是 `transcript.rs`（那个模块只管转写面本身）。
//!
//! ## 与收口前的三点差别
//!
//! 1. **零 serde**：事件从生产者出来就是类型化的 `NodeOp`，直达唯一写入点，
//!    不再「序列化成帧 → 反序列化回来只为分辨变体」；
//! 2. **没有解析失败分支**：类型化之后不存在「无法解析的数据帧」这种降级路径；
//! 3. **写入不再与消费循环同寿**：生产者（`run_chat_loop` 任务）直接写，
//!    消费循环因此可以退化为纯生命周期管理（见 [`super::consume`]）。

use super::*;
use crate::symbio_core::exec::TranscriptWriter;
use crate::symbio_core::schemas::session::session_chat_response::NodeOp;
use async_trait::async_trait;

/// 执行期事件出口：直连转写唯一写入点 + 会话级状态分派。
pub(crate) struct TranscriptSink {
    plugin: Arc<SessionPlugin>,
    state: Arc<ActiveSessionState>,
}

impl TranscriptSink {
    pub(crate) fn new(plugin: Arc<SessionPlugin>, state: Arc<ActiveSessionState>) -> Self {
        Self { plugin, state }
    }
}

#[async_trait]
impl TranscriptWriter for TranscriptSink {
    async fn apply(&self, op: NodeOp) {
        match op {
            // 会话级告警（持久化失败 / 长度截断 / 工具轮次上限）：
            // 直通会话节点 `attributes.warning`（VDFS watch 域，状态非事件），
            // **不进转写**——与收口前消费循环的分派逐字一致。
            NodeOp::Warn { warning } => {
                self.plugin
                    .emit_session_state(&self.state, SessionStateChange::Warning(warning))
                    .await;
            }
            // 消息级变更：进唯一写入点（内存图 → seq → 一行核心日志 → 发布）。
            op => {
                self.state.transcript.lock().await.apply(op);
            }
        }
    }
}
