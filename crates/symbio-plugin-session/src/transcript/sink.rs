//! 执行期出口的**生产实现**：把模型 / 工具的流式帧写回本会话转写。
//!
//! 内核出口（[`symbio_kernel::exec::ExecEventSink`]）只约定「一条消息帧」与
//! 「转写唯一写入点」的抽象；本文件把那个抽象接到 [`ActiveSessionManager`]——
//! 与会话编排、压缩发射器用的是同一张名册、同一条 `Transcript::apply` 路径，
//! 因此实时面（`vdfs/watch`）不需要第二条代码路径。
//!
//! 出口的**打开归属**：由会话插件在自己的部署根上按会话构造，随事实源一起
//! 交给装配层的驱动；装配层原样塞进 `PortExecHandles.sink`，不再写死静默。

use std::sync::Arc;

use symbio_kernel::chat::chat_node as cm;
use symbio_kernel::exec::ExecTranscriptWriter;

use crate::active::ActiveSessionManager;

/// 按会话绑定的转写出口。
pub(crate) struct TranscriptSink {
    manager: Arc<ActiveSessionManager>,
    session_id: String,
}

impl TranscriptSink {
    pub(crate) fn new(manager: Arc<ActiveSessionManager>, session_id: String) -> Self {
        Self {
            manager,
            session_id,
        }
    }
}

#[async_trait::async_trait]
impl ExecTranscriptWriter for TranscriptSink {
    async fn apply(&self, message: cm::ChatNode) {
        self.manager
            .transcript_apply(&self.session_id, message)
            .await;
    }
}
