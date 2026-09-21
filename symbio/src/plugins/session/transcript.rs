//! 会话转写（Transcript）：实时链路的**唯一写入点与发射器**（session 域内）。
//!
//! - **唯一写入点**：一切消息级变更（模型流式 / 工具执行 / 恢复重写 / 压缩 /
//!   用户消息定稿）都以 [`NodeOp`] 进入 [`Transcript::apply`]——内存图、seq、
//!   核心日志、发布四件事在同一个函数里完成，没有任何旁路。
//! - **单调 seq**：每发布一帧 seq +1。消费端检测到缺口 = 丢帧当场可见，
//!   唯一恢复路径是清空本地转写并从存储整份重读。
//! - **核心日志**：每帧一行，即时间线本身——故障排查从"全栈考古"变成"读时间线"。
//! - **显式背压**：见 `symbio_core::transcript_stream`（满即踢 → 泵 EOF → 整份重读）。
//!
//! ## 与 VDFS 的边界
//!
//! 消息的**实时面**走转写流；**历史面**（落库转写、`消息` 目录的 list / read
//! 投影）仍是 VDFS。会话节点自身的状态（working / error / warning）也仍走
//! VDFS watch（低频、非突发，且与会话清单共用同一订阅）。

use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::schemas::session::session_chat_response::NodeOp;
use crate::symbio_core::transcript_stream::{publish_frame, NodeEvent};
use crate::{plugin_error, plugin_info, plugin_warn};
use indexmap::IndexMap;

/// 会话转写：内存图（在途视图）+ 单调 seq + 发布。
///
/// 挂在 [`crate::plugins::session::active::ActiveSessionState`] 上（每会话一个），
/// 消费循环是它唯一的常规写入者；`emit_persisted_message`（用户消息定稿）与
/// 压缩发射器经同一入口写入。
pub struct Transcript {
    session_id: String,
    seq: u64,
    nodes: IndexMap<String, cm::ChatMessage>,
}

impl Transcript {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            seq: 0,
            nodes: IndexMap::new(),
        }
    }

    /// 唯一入口：更新内存图 → 分配 seq → 一行核心日志 → 发布。
    ///
    /// 协议违例（对未知 id 追加）报错丢弃：不发布、不占 seq——发布出去的帧
    /// 严格连续，消费端的缺口检测因此不被违例帧污染。
    pub fn apply(&mut self, op: NodeOp) {
        match op {
            NodeOp::Upsert { message } => {
                let existed = self
                    .nodes
                    .insert(message.id.clone(), (*message).clone())
                    .is_some();
                // Start / Update / End 是图内状态的机械判别，只用于日志——
                // 线上只有一种全量帧，没有第二种解释。
                let phase = match (existed, message.status.as_ref().map(|s| s.as_str())) {
                    (false, _) => "Start",
                    (true, Some(s)) if is_terminal(s) => "End",
                    (true, _) => "Update",
                };
                self.emit(NodeOp::Upsert { message }, phase.to_string());
            }
            NodeOp::Append { message_id, delta } => match self.nodes.get_mut(&message_id) {
                Some(existing) => match existing.content.as_mut() {
                    Some(cm::MessageContent::Text(buf)) => {
                        buf.push_str(&delta);
                        self.emit(
                            NodeOp::Append {
                                message_id,
                                delta: delta.clone(),
                            },
                            format!("+{}c", delta.chars().count()),
                        );
                    }
                    _ => {
                        plugin_error!(
                            "session",
                            "[Transcript] 协议违例：Append 目标 {} 的内容不是 Text，帧丢弃",
                            message_id
                        );
                    }
                },
                None => {
                    plugin_error!(
                        "session",
                        "[Transcript] 协议违例：对未知 id `{message_id}` 追加（created 丢失），帧丢弃"
                    );
                }
            },
            NodeOp::Remove { message_id } => {
                self.nodes.shift_remove(&message_id);
                self.emit(NodeOp::Remove { message_id }, String::new());
            }
            NodeOp::Reset => {
                let n = self.nodes.len();
                self.nodes.clear();
                self.emit(NodeOp::Reset, format!("{n} nodes"));
            }
            // Warn 是会话级状态，由消费循环直通会话节点（emit_session_state），
            // 不经本函数。防御性兜底：记录并忽略，不发布。
            NodeOp::Warn { .. } => {
                plugin_warn!(
                    "session",
                    "[Transcript] Warn 不应进入 Transcript::apply（会话级状态走会话节点），已忽略"
                );
            }
        }
    }

    /// 落库回执：权威副本已写入存储，把节点从内存图移除（不发布）。
    ///
    /// 存储是唯一权威；图里只留**在途**节点（VDFS 转写列表的叠加来源）。
    pub fn persisted(&mut self, ids: &[String]) {
        for id in ids {
            self.nodes.shift_remove(id);
        }
    }

    /// 在途图快照（VDFS 转写列表的叠加源）。
    pub fn snapshot(&self) -> Vec<cm::ChatMessage> {
        self.nodes.values().cloned().collect()
    }

    /// 取单个在途节点。
    pub fn get(&self, id: &str) -> Option<cm::ChatMessage> {
        self.nodes.get(id).cloned()
    }

    /// 清空在途图（轮次收尾：权威副本已全部落库）。
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    /// 分配 seq、打一行核心日志、发布。seq 只在被发布的帧上消耗。
    fn emit(&mut self, op: NodeOp, detail: String) {
        self.seq += 1;
        let seq = self.seq;
        let (kind, id, status) = describe(&op);
        plugin_info!("session", "[T#{seq} {kind}] {id} {status} {detail}");
        publish_frame(&NodeEvent {
            session_id: self.session_id.clone(),
            seq,
            op,
        });
    }
}

/// 日志用摘要：操作名 + 消息 id + 状态。
fn describe(op: &NodeOp) -> (&'static str, String, String) {
    match op {
        NodeOp::Upsert { message } => (
            "Upsert",
            message.id.clone(),
            message
                .status
                .as_ref()
                .map(|s| s.as_str().to_string())
                .unwrap_or_else(|| "-".into()),
        ),
        NodeOp::Append { message_id, .. } => ("Append", message_id.clone(), "-".into()),
        NodeOp::Remove { message_id } => ("Remove", message_id.clone(), "-".into()),
        NodeOp::Reset => ("Reset", "-".into(), "-".into()),
        NodeOp::Warn { .. } => ("Warn", "-".into(), "-".into()),
    }
}

/// 终态判定（日志用）：Streaming / Pending 之外的已落状态词。
fn is_terminal(status: &str) -> bool {
    !matches!(status, "streaming" | "pending")
}

#[cfg(test)]
#[path = "transcript.test.rs"]
mod tests;
