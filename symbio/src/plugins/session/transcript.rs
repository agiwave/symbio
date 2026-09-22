//! 会话转写（Transcript）：实时链路的**唯一写入点与发射器**（session 域内）。
//!
//! - **唯一写入点**：一切消息级变更（模型流式 / 工具执行 / 恢复重写 / 压缩 /
//!   用户消息定稿）都以**一条消息帧**（[`cm::ChatMessage`]）进入
//!   [`Transcript::apply`]——内存图、seq、核心日志、发布四件事在同一个函数里
//!   完成，没有任何旁路。
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
use crate::symbio_core::transcript_stream::{publish_frame, NodeEvent};
use crate::{plugin_error, plugin_info};
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
    /// ## 一帧就是一条消息，语义全在字段上
    ///
    /// | 帧里有什么 | 图里发生什么 |
    /// |---|---|
    /// | `delta` | 追加到该节点正文尾部 |
    /// | `content` | 整条替换该节点正文（幂等） |
    /// | `status = removed` | 就地移除该节点（不落图） |
    /// | `status`（其余） | 状态迁移 |
    /// | 身份字段 / `meta` / `seq` / `timestamp` | 有则覆盖（发射端持有当前完整值） |
    ///
    /// 未知 id 用帧内信息建占位再合并——帧自给自足，不依赖任何先行帧。
    ///
    /// ## 协议违例：同帧既带增量又带完整正文
    ///
    /// 该拼接还是该替换？语义不可判定，报错丢弃：不发布、不占 seq——发布出去的
    /// 帧严格连续，消费端的缺口检测因此不被违例帧污染。
    pub fn apply(&mut self, msg: cm::ChatMessage) {
        if msg.delta.is_some() && msg.content.is_some() {
            plugin_error!(
                "session",
                "[Transcript] 协议违例：节点 {} 同帧携带 delta 与 content（增量/全量语义互斥），帧丢弃",
                msg.id
            );
            return;
        }

        let message_id = msg.id.clone();

        // 删除：不落图，就地广播该状态帧（帧照常占 seq，「删了什么」在时间线上可追溯）。
        if msg.status == Some(cm::MessageStatus::Removed) {
            let detail = self
                .nodes
                .shift_remove(&message_id)
                .map(|n| n.name.unwrap_or_else(|| n.id.clone()))
                .unwrap_or_else(|| "（不在途）".into());
            self.emit(msg, format!("Removed {detail}"));
            return;
        }

        let existed = self.nodes.contains_key(&message_id);
        let node = self.nodes.entry(message_id.clone()).or_insert_with(|| {
            // 未知 id：用帧内身份建占位（帧自给自足）。
            cm::ChatMessage {
                id: message_id.clone(),
                ..Default::default()
            }
        });

        // 身份字段：有则覆盖（首帧建立身份；重复携带以最后到达为准）。
        if msg.parent_id.is_some() {
            node.parent_id = msg.parent_id.clone();
        }
        if msg.role.is_some() {
            node.role = msg.role.clone();
        }
        if msg.msg_type.is_some() {
            node.msg_type = msg.msg_type.clone();
        }
        if msg.name.is_some() {
            node.name = msg.name.clone();
        }
        if msg.tool_call_id.is_some() {
            node.tool_call_id = msg.tool_call_id.clone();
        }
        if msg.meta.is_some() {
            node.meta = msg.meta.clone();
        }
        // 存储侧权威值（落库后回发的对齐帧）。
        if msg.seq.is_some() {
            node.seq = msg.seq;
        }
        if msg.timestamp.is_some() {
            node.timestamp = msg.timestamp;
        }
        // 状态迁移。
        if msg.status.is_some() {
            node.status = msg.status.clone();
        }
        // 错误信息：有则覆盖（None 不清除——清除走 status 不是 Failed 的终态帧）。
        if msg.error.is_some() {
            node.error = msg.error.clone();
        }

        // 正文：增量追加 / 完整替换，互斥（违例已在上面拒绝）。
        // 图里只留**累积后的 `content`**：`delta` 是传输形态，不是节点形态。
        if let Some(delta) = &msg.delta {
            match node.content.as_mut() {
                Some(cm::MessageContent::Text(buf)) => buf.push_str(delta),
                _ => node.content = Some(cm::MessageContent::Text(delta.clone())),
            }
        } else if let Some(content) = &msg.content {
            node.content = Some(content.clone());
        }

        // 日志相位：出现（首帧建占位）/ 更新 / 终态（机械判别，仅用于日志）。
        let phase = match (existed, msg.status.as_ref().map(|s| s.as_str())) {
            (false, _) => "Start",
            (true, Some(s)) if is_terminal(s) => "End",
            _ => "Update",
        };
        let detail = match (&msg.delta, &msg.content, &msg.status) {
            (Some(d), _, Some(s)) => format!("+{}c → {}", d.chars().count(), s.as_str()),
            (Some(d), _, None) => format!("+{}c", d.chars().count()),
            (None, Some(c), _) => format!("={}c", c.len()),
            (None, None, Some(s)) => s.as_str().to_string(),
            (None, None, None) => String::new(),
        };
        self.emit(
            msg,
            if detail.is_empty() {
                phase.to_string()
            } else {
                format!("{phase} {detail}")
            },
        );
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
    fn emit(&mut self, message: cm::ChatMessage, detail: String) {
        self.seq += 1;
        let seq = self.seq;
        let status = message
            .status
            .as_ref()
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "-".into());
        plugin_info!("session", "[T#{seq}] {} {status} {detail}", message.id);
        publish_frame(&NodeEvent {
            session_id: self.session_id.clone(),
            seq,
            message,
        });
    }
}

/// 终态判定（日志用）：Streaming / Pending 之外的已落状态词。
fn is_terminal(status: &str) -> bool {
    !matches!(status, "streaming" | "pending")
}

#[cfg(test)]
#[path = "transcript.test.rs"]
mod tests;
