//! 会话转写（Transcript）：实时链路的**唯一写入点与发射器**（session 域内）。
//!
//! - **唯一写入点**：一切消息级变更（模型流式 / 工具执行 / 恢复重写 / 压缩 /
//!   用户消息定稿）都以 [`NodeOp`] 进入 [`Transcript::apply`]——内存图、seq、
//!   核心日志、发布四件事在同一个函数里完成，没有任何旁路。
//! - **单调 seq**：每发布一帧 seq +1。消费端检测到缺口 = 丢帧当场可见，
//!   唯一恢复路径是清空本地转写并从存储整份重读。
//! - **核心日志**：每帧一行，即时间线本身——故障排查从"全栈考古"变成"读时间线"。
//!   **但连续的同节点 Append 折成一行**（见 [`AppendLogCoalescer`]）：一次流式回复
//!   有几百个增量帧，逐帧一行会把时间线淹成噪声（实测一段 200 字回复 = 580 行
//!   `+Nc`，占该轮 stderr 的 93%）。折行**只作用于日志**——seq 照旧逐帧分配、
//!   帧照旧逐帧发布，实时链路一个字节都不变。
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
    /// 日志合并器（**只影响日志**，不参与 seq 与发布）。
    append_log: AppendLogCoalescer,
}

/// 连续同节点 `Append` 的**日志**合并器。
///
/// ## 为什么需要它
///
/// 一次流式回复的增量帧数由模型决定，实测几百帧（`+1c` / `+2c` / `+7c` …）。
/// 逐帧一行的结果是**时间线被同一件事填满**——而它们本就是一件事：某个节点在增长。
/// 折行后一次回复的日志从 580 行降到个位数，`Upsert` / `Remove` / `Reset` 这些
/// 真正的状态变更才重新可见。
///
/// ## 边界：只折日志
///
/// `seq` 是**协议**（消费端按它检测丢帧），发布是**数据面**。两者都必须逐帧进行，
/// 因此本结构体只被 `Transcript::emit` 用于"要不要打这一行、打成什么样"。
///
/// ## 折行规则
///
/// - 相邻帧**同 `message_id`** 且都是 `Append` → 并入本 run，不产出日志行；
/// - 换 id、换操作、或显式 `flush`（`persisted` / `clear` / `Warn`）→ 冲刷本 run。
///
/// 单帧 run 渲染成与折行前**逐字相同**的行（`[T#7 Append] id - +3c`），
/// 保证"只有一帧"这种常见情形下日志形态不变；多帧才用带区间与统计的形状。
#[derive(Debug, Default)]
pub(crate) struct AppendLogCoalescer {
    run: Option<AppendRun>,
}

/// 一段连续的同节点 `Append` 的累计量。
#[derive(Debug, PartialEq, Eq)]
struct AppendRun {
    message_id: String,
    first_seq: u64,
    last_seq: u64,
    frames: u64,
    chars: usize,
}

impl AppendLogCoalescer {
    /// 喂一帧。返回**需要立刻打出的行**：
    ///
    /// - 本帧是 Append 且与本 run 同 id → `None`（已并入，不产出）；
    /// - 本帧是 Append 但换了 id → 上一 run 的冲刷行（若上一 run 存在）；
    /// - 本帧不是 Append → 上一 run 的冲刷行（若存在）。
    ///
    /// 非 Append 帧自己的日志行由调用方负责（它带 `kind` / `status`，不属于本器）。
    pub(crate) fn feed(&mut self, op: &NodeOp, seq: u64) -> Option<String> {
        let NodeOp::Append { message_id, delta } = op else {
            return self.flush();
        };
        let chars = delta.chars().count();
        if let Some(run) = self.run.as_mut() {
            if run.message_id == *message_id {
                run.last_seq = seq;
                run.frames += 1;
                run.chars += chars;
                return None;
            }
        }
        // 换了 id：先冲刷旧 run，再为本帧开新 run。
        let flushed = self.flush();
        self.run = Some(AppendRun {
            message_id: message_id.clone(),
            first_seq: seq,
            last_seq: seq,
            frames: 1,
            chars,
        });
        flushed
    }

    /// 冲刷待合并的 run。没有待合并时返回 `None`。
    ///
    /// 调用点：换 id / 换操作（`feed` 内）、`persisted`（落库回执 = 这段增长结束）、
    /// `clear`（轮次收尾）、`Warn`（防御性分支，不经 `emit`）。
    pub(crate) fn flush(&mut self) -> Option<String> {
        let run = self.run.take()?;
        Some(run.render())
    }
}

impl AppendRun {
    /// 渲染成一行核心日志。
    fn render(&self) -> String {
        if self.frames == 1 {
            // 单帧：与折行前的形状逐字相同
            format!(
                "[T#{} Append] {} - +{}c",
                self.first_seq, self.message_id, self.chars
            )
        } else {
            format!(
                "[T#{}..{} Append] {} - {} 帧 / +{}c",
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
            append_log: AppendLogCoalescer::default(),
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
                // 冲刷待合并的 Append run：Warn 之后通常就是收尾，别把最后一段吞掉。
                if let Some(line) = self.append_log.flush() {
                    plugin_info!("session", "{line}");
                }
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
        // 落库 = 该节点的增长段结束，把待合并的 Append run 收尾。
        if let Some(line) = self.append_log.flush() {
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
    /// 顺带冲刷待合并的 Append run——轮次边界是"这一段增长结束了"的最强信号，
    /// 也是最后一段 Append 日志不至于被吞掉的保证（`clear` 在轮次起止各调一次）。
    pub fn clear(&mut self) {
        if let Some(line) = self.append_log.flush() {
            plugin_info!("session", "{line}");
        }
        self.nodes.clear();
    }

    /// 分配 seq、打一行核心日志、发布。seq 只在被发布的帧上消耗。
    ///
    /// ## 日志与发布在这里分岔（唯一一处）
    ///
    /// `seq` 分配与 `publish_frame` **逐帧无例外**——它们是协议与数据面。
    /// 只有**日志**会被 [`AppendLogCoalescer`] 折行：连续同节点的 `Append`
    /// 合并成一行统计（`[T#4..583 Append] id - 580 帧 / +1234c`）。
    /// 折行前后的可观测差异**只有 stderr 的行数**。
    fn emit(&mut self, op: NodeOp, detail: String) {
        self.seq += 1;
        let seq = self.seq;

        // 折行器先吃这一帧：可能冲刷出上一段 Append 的统计行。
        if let Some(line) = self.append_log.feed(&op, seq) {
            plugin_info!("session", "{line}");
        }
        // 非 Append 帧自己的行（带 kind / status，不属于折行器的职责）。
        if !matches!(op, NodeOp::Append { .. }) {
            let (kind, id, status) = describe(&op);
            plugin_info!("session", "[T#{seq} {kind}] {id} {status} {detail}");
        }

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
