//! 会话转写（Transcript）：实时链路的**唯一写入点与发射器**（session 域内）。
//!
//! - **唯一写入点**：一切消息级变更（模型流式 / 工具执行 / 恢复重写 / 压缩 /
//!   用户消息定稿）都以**一条消息帧**（[`cm::ChatMessage`]）进入
//!   [`Transcript::apply`]——内存图、seq、核心日志、发布四件事在同一个函数里
//!   完成，没有任何旁路。
//! - **单调 seq**：每发布一帧 seq +1。消费端检测到缺口 = 丢帧当场可见，
//!   唯一恢复路径是清空本地转写并从存储整份重读。
//! - **核心日志**：每帧一行，即时间线本身——故障排查从"全栈考古"变成"读时间线"。
//!   **但连续的、同一节点的纯流式增量（帧带 `delta`、无状态迁移）折成一行**
//!   （见 [`DeltaLogCoalescer`]）：一次流式回复有几百个增量帧，逐帧一行会把时间线
//!   淹成噪声（实测一段 200 字回复 = 580 行 `+Nc`，占该轮 stderr 的 93%）。折行
//!   **只作用于日志**——seq 照旧逐帧分配、帧照旧逐帧发布，实时链路一个字节都不变。
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
    /// 日志合并器（**只影响日志**，不参与 seq 与发布）。
    delta_log: DeltaLogCoalescer,
}

/// 连续同节点**纯流式增量**（帧带 `delta`、无状态迁移）的**日志**合并器。
///
/// ## 为什么需要它
///
/// 一次流式回复的增量帧数由模型决定，实测几百帧（`+1c` / `+2c` / `+7c` …）。
/// 逐帧一行的结果是**时间线被同一件事填满**——而它们本就是一件事：某个节点在增长。
/// 折行后一次回复的日志从几百行降到个位数，`Start` / `End` / `Removed` 这些
/// 真正的状态迁移才重新可见。
///
/// ## 边界：只折日志
///
/// `seq` 是**协议**（消费端按它检测丢帧），发布是**数据面**。两者都必须逐帧进行，
/// 因此本结构体只被 `Transcript::emit` 用于"要不要打这一行、打成什么样"。
///
/// ## 折行规则
///
/// - 相邻帧**同 `message.id`** 且都是**纯增量**（`delta` 有、`status` 无）→ 并入本 run，
///   不产出日志行；
/// - 换 id、或该帧带了状态迁移 / 全量正文（非纯增量）→ 冲刷本 run；
/// - 显式 `flush`（`persisted` / `clear`）也冲刷。
///
/// 单帧 run 渲染成与折行前**逐字相同**的行（`[T#7] id - Update +3c`），
/// 保证"只有一帧"这种常见情形下日志形态不变；多帧才用带区间与统计的形状。
#[derive(Debug, Default)]
pub(crate) struct DeltaLogCoalescer {
    run: Option<DeltaRun>,
}

/// 一段连续的同节点纯增量的累计量。
#[derive(Debug, PartialEq, Eq)]
struct DeltaRun {
    message_id: String,
    first_seq: u64,
    last_seq: u64,
    frames: u64,
    chars: usize,
}

impl DeltaLogCoalescer {
    /// 喂一帧。返回 `(需立刻打出的上一段统计行, 本帧是否被并入)`。
    ///
    /// - 本帧是**纯增量**（带 `delta`、无 `status`）且与本 run 同 id → 并入，`(None, true)`；
    /// - 本帧是纯增量但换了 id → 冲刷上一段，并为新 id 开新 run，`(flushed, true)`；
    /// - 本帧非纯增量（状态迁移 / 全量正文 / 仅身份）→ 冲刷上一段，本帧**不**并入，
    ///   `(flushed, false)`——它自己的日志行由调用方照常打印。
    pub(crate) fn feed(&mut self, msg: &cm::ChatMessage, seq: u64) -> (Option<String>, bool) {
        let chars = match (&msg.delta, &msg.status) {
            (Some(d), None) => d.chars().count(),
            _ => return (self.flush(), false),
        };
        if let Some(run) = self.run.as_mut() {
            if run.message_id == msg.id {
                run.last_seq = seq;
                run.frames += 1;
                run.chars += chars;
                return (None, true);
            }
        }
        // 换了 id：先冲刷旧 run，再为本帧开新 run。
        let flushed = self.flush();
        self.run = Some(DeltaRun {
            message_id: msg.id.clone(),
            first_seq: seq,
            last_seq: seq,
            frames: 1,
            chars,
        });
        (flushed, true)
    }

    /// 冲刷待合并的 run。没有待合并时返回 `None`。
    ///
    /// 调用点：换 id / 非纯增量帧（`feed` 内）、`persisted`（落库回执 = 这段增长结束）、
    /// `clear`（轮次收尾，保证最后一段不被吞掉）。
    pub(crate) fn flush(&mut self) -> Option<String> {
        let run = self.run.take()?;
        Some(run.render())
    }
}

impl DeltaRun {
    /// 渲染成一行核心日志。
    fn render(&self) -> String {
        if self.frames == 1 {
            // 单帧：与折行前的形状逐字相同
            format!(
                "[T#{}] {} - Update +{}c",
                self.first_seq, self.message_id, self.chars
            )
        } else {
            format!(
                "[T#{}..{}] {} - Update {} 帧 / +{}c",
                self.first_seq, self.last_seq, self.message_id, self.frames, self.chars
            )
        }
    }
}

impl Transcript {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            seq: 0,
            nodes: IndexMap::new(),
            delta_log: DeltaLogCoalescer::default(),
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
            self.emit(msg, detail);
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

        // 纯增量帧会被折行器并入（`emit` 里省略自己的行），其 `detail` 不被读取——
        // 跳过构造，免得每个 token 白做一次 `format!`。
        let detail = if msg.delta.is_some() && msg.status.is_none() {
            String::new()
        } else {
            // 日志相位：出现（首帧建占位）/ 更新 / 终态（机械判别，仅用于日志）。
            let phase = match (existed, msg.status.as_ref().map(|s| s.as_str())) {
                (false, _) => "Start",
                (true, Some(s)) if is_terminal(s) => "End",
                _ => "Update",
            };
            // `detail` **不重复 status**（它已单独成列），只描述正文形态：
            // `+Nc` 增量 / `=Nc` 全量 / 空（仅状态或仅身份）。`delta` 与 `content`
            // 同帧已在前面拒绝，故二者不会同时出现。
            let detail = match (&msg.delta, &msg.content) {
                (Some(d), _) => format!("+{}c", d.chars().count()),
                (None, Some(c)) => format!("={}c", c.len()),
                (None, None) => String::new(),
            };
            if detail.is_empty() {
                phase.to_string()
            } else {
                format!("{phase} {detail}")
            }
        };
        self.emit(msg, detail);
    }

    /// 落库回执：权威副本已写入存储，把节点从内存图移除（不发布）。
    ///
    /// 存储是唯一权威；图里只留**在途**节点（VDFS 转写列表的叠加来源）。
    pub fn persisted(&mut self, ids: &[String]) {
        // 落库 = 该节点的增长段结束，把待合并的增量 run 收尾。
        if let Some(line) = self.delta_log.flush() {
            plugin_info!("session", "{line}");
        }
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
    ///
    /// 顺带冲刷待合并的增量 run——轮次边界是"这一段增长结束了"的最强信号，
    /// 也是最后一段增量日志不至于被吞掉的保证（`clear` 在轮次起止各调一次）。
    pub fn clear(&mut self) {
        if let Some(line) = self.delta_log.flush() {
            plugin_info!("session", "{line}");
        }
        self.nodes.clear();
    }

    /// 分配 seq、打一行核心日志、发布。seq 只在被发布的帧上消耗。
    ///
    /// ## 日志与发布在这里分岔（唯一一处）
    ///
    /// `seq` 分配与 `publish_frame` **逐帧无例外**——它们是协议与数据面。
    /// 只有**日志**会被 [`DeltaLogCoalescer`] 折行：连续同节点的纯流式增量合并成
    /// 一行统计（`[T#4..583] id - Update 580 帧 / +1234c`）。折行前后的可观测差异
    /// **只有 stderr 的行数**。
    ///
    /// 被并入的纯增量帧其 `detail` 不被读取（调用方传空串即可）。
    fn emit(&mut self, message: cm::ChatMessage, detail: String) {
        self.seq += 1;
        let seq = self.seq;

        // 折行器先吃这一帧：可能冲刷出上一段纯增量的统计行。
        let (flushed, absorbed) = self.delta_log.feed(&message, seq);
        if let Some(line) = flushed {
            plugin_info!("session", "{line}");
        }
        // 被并入的纯增量帧不产生自己的行；其余帧照常打印（带 status 列与 detail）。
        if !absorbed {
            let status = message.status.as_ref().map(|s| s.as_str()).unwrap_or("-");
            plugin_info!("session", "[T#{seq}] {} {status} {detail}", message.id);
        }

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
