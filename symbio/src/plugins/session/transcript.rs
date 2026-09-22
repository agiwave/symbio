//! 会话转写（Transcript）：实时链路的**唯一写入点与发射器**（session 域内）。
//!
//! - **唯一写入点**：一切消息级变更（模型流式 / 工具执行 / 恢复重写 / 压缩 /
//!   用户消息定稿）都以**一条消息帧**（[`cm::ChatMessage`]）进入
//!   [`Transcript::apply`]——内存图、seq、核心日志、发布四件事在同一个函数里
//!   完成，没有任何旁路。
//! - **单调 seq**：每发布一帧 seq +1。消费端检测到缺口 = 丢帧当场可见，
//!   唯一恢复路径是清空本地转写并从存储整份重读。
//! - **核心日志**：分**骨架**与**细节**两级（见 [`FrameLogLevel`]）。骨架 = 节点
//!   出现 / 终态 / 等待用户 / 删除，进 `INFO`——"什么出现了、什么时候结束"一眼看完；
//!   细节 = `Update` 帧与纯增量的折行统计，进 `DEBUG`（`--verbose` 或
//!   `SYMBIO_LOG=debug` 放开）。细节里的**连续同节点纯增量**（帧带 `delta`、无状态
//!   迁移）再被 [`DeltaLogCoalescer`] 折成一行统计：一次流式回复有几百个增量帧，
//!   逐帧一行会把时间线淹成噪声（实测一段 200 字回复 = 580 行 `+Nc`，占该轮 stderr
//!   的 93%）。分级与折行**只作用于日志**——seq 照旧逐帧分配、帧照旧逐帧发布，
//!   实时链路一个字节都不变（不变量 #28）。
//! - **显式背压**：见 `symbio_core::transcript_stream`（满即踢 → 泵 EOF → 整份重读）。
//!
//! ## 与 VDFS 的边界
//!
//! 消息的**实时面**走转写流；**历史面**（落库转写、`消息` 目录的 list / read
//! 投影）仍是 VDFS。会话节点自身的状态（working / error / warning）也仍走
//! VDFS watch（低频、非突发，且与会话清单共用同一订阅）。

use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::transcript_stream::{publish_frame, NodeEvent};
use crate::{plugin_debug, plugin_error, plugin_info};
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
/// 折行后一次回复的增量日志从几百行降到个位数，骨架帧才重新可见。
///
/// ## 边界：只折日志
///
/// `seq` 是**协议**（消费端按它检测丢帧），发布是**数据面**。两者都必须逐帧进行，
/// 因此本结构体只被 `Transcript::emit` 用于"要不要打这一行、打成什么样"。
///
/// ## 折行规则
///
/// - 相邻帧**同 `message.id`**、都是**纯增量**（`delta` 有、`status` 无）、**且允许
///   折行**（`foldable`）→ 并入本 run，不产出日志行；
/// - 换 id、或该帧带了状态迁移 / 全量正文（非纯增量）、或**不被允许折行** → 冲刷本 run；
/// - 显式 `flush`（`persisted` / `clear`）也冲刷。
///
/// `foldable` 由调用方给出，它恒等于"这一帧是 [`FrameLogLevel::Detail`]"——**骨架帧
/// 一律不可折**（§10 的"状态帧不折"）。首帧尤其重要：即便它恰好是纯增量，"某节点
/// 何时出现"这条骨架也必须留下，否则时间线会缺掉一个节点的起点。
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
    /// `foldable`：本帧**是否允许折行**（骨架帧为 `false`，见结构体文档）。
    ///
    /// - 本帧是**纯增量**且 `foldable`、与本 run 同 id → 并入，`(None, true)`；
    /// - 本帧是纯增量且 `foldable` 但换了 id → 冲刷上一段，并为新 id 开新 run，`(flushed, true)`；
    /// - 其余（非纯增量 / 骨架帧）→ 冲刷上一段，本帧**不**并入，`(flushed, false)`——
    ///   它自己的日志行由调用方照常打印。
    pub(crate) fn feed(
        &mut self,
        msg: &cm::ChatMessage,
        seq: u64,
        foldable: bool,
    ) -> (Option<String>, bool) {
        let chars = match (&msg.delta, &msg.status) {
            (Some(d), None) if foldable => d.chars().count(),
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

/// 核心日志的两级：**骨架**（`INFO`）与**细节**（`DEBUG`）。
///
/// 一条时间线只有骨架值得常看；其余都是"怎么长起来的"过程量，需要时再放开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameLogLevel {
    /// **关键节点**：节点出现 / 终态 / 等待用户 / 删除。进 `INFO`，且**不可折行**。
    Skeleton,
    /// **过程细节**：`Update` 帧（`pending → streaming`、正文替换、仅 `meta` 变更）与
    /// 纯增量的折行统计。进 `DEBUG`；其中的纯增量帧可被折行器并入。
    Detail,
}

/// 一帧的日志规格（`apply` 判好、`emit` 执行——`emit` 不再反推语义）。
struct FrameLog {
    level: FrameLogLevel,
    /// 一行的正文描述：`<相位> [<正文形态>]`；可能被折行的帧传空串（不被读取）。
    detail: String,
}

/// 帧在日志里的**相位与级别**（机械判别，只用于日志）。
///
/// | 相位 | 条件 | 级别 |
/// |---|---|---|
/// | `Start` | 节点在帧前不在内存图中 | 骨架 |
/// | `Wait` | 迁到 `waiting_user_action` | 骨架（**要人介入，必须可见**） |
/// | `End` | 迁到任一终态词 | 骨架 |
/// | `Update` | 其余（含 `pending → streaming`、正文替换、仅 `meta` 变更） | 细节 |
///
/// `Removed` 实际走 [`Transcript::apply`] 的早退分支、到不了这里；把它留在 `End` 一组
/// 是为了让"终态词"这个集合在本函数内**完整**——漏掉一个终态词就会把它错判成过程量。
fn frame_log_of(
    existed: bool,
    status: Option<&cm::MessageStatus>,
) -> (&'static str, FrameLogLevel) {
    if !existed {
        return ("Start", FrameLogLevel::Skeleton);
    }
    match status {
        Some(cm::MessageStatus::WaitingUserAction) => ("Wait", FrameLogLevel::Skeleton),
        Some(
            cm::MessageStatus::Completed
            | cm::MessageStatus::Failed
            | cm::MessageStatus::Aborted
            | cm::MessageStatus::Removed,
        ) => ("End", FrameLogLevel::Skeleton),
        _ => ("Update", FrameLogLevel::Detail),
    }
}

/// 渲染一行核心日志：`[T#<seq>] <id> <status> [<detail>]`。
///
/// 无 `detail` 时不落尾随空格——日志要能直接复制粘贴、能整齐对齐。
fn render_frame_line(seq: u64, id: &str, status: &str, detail: &str) -> String {
    if detail.is_empty() {
        format!("[T#{seq}] {id} {status}")
    } else {
        format!("[T#{seq}] {id} {status} {detail}")
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
            self.emit(
                msg,
                FrameLog {
                    level: FrameLogLevel::Skeleton,
                    detail,
                },
            );
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

        // 日志相位与级别（机械判别，只用于日志）：出现 / 终态 / 等待用户是**骨架**，
        // 其余是**细节**。
        let (phase, level) = frame_log_of(existed, msg.status.as_ref());
        // 细节里的纯增量帧会被折行器并入（`emit` 里不留自己的行），其 `detail` 不被
        // 读取——跳过构造，免得每个 token 白做一次 `format!`（骨架帧绝不跳过：首帧
        // 即使恰好是纯增量也要报出它的正文形态）。
        let detail =
            if level == FrameLogLevel::Detail && msg.delta.is_some() && msg.status.is_none() {
                String::new()
            } else {
                // `detail` **不重复 status**（它已单独成列），只描述正文形态：
                // `+Nc` 增量 / `=Nc` 全量 / 空（仅状态或仅身份）。`delta` 与 `content`
                // 同帧已在前面拒绝，故二者不会同时出现。
                let shape = match (&msg.delta, &msg.content) {
                    (Some(d), _) => format!("+{}c", d.chars().count()),
                    (None, Some(c)) => format!("={}c", c.len()),
                    (None, None) => String::new(),
                };
                if shape.is_empty() {
                    phase.to_string()
                } else {
                    format!("{phase} {shape}")
                }
            };
        self.emit(msg, FrameLog { level, detail });
    }

    /// 落库回执：权威副本已写入存储，把节点从内存图移除（不发布）。
    ///
    /// 存储是唯一权威；图里只留**在途**节点（VDFS 转写列表的叠加来源）。
    pub fn persisted(&mut self, ids: &[String]) {
        // 落库 = 该节点的增长段结束，把待合并的增量 run 收尾。
        if let Some(line) = self.delta_log.flush() {
            plugin_debug!("session", "{line}");
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
            plugin_debug!("session", "{line}");
        }
        self.nodes.clear();
    }

    /// 分配 seq、打一行核心日志、发布。seq 只在被发布的帧上消耗。
    ///
    /// ## 日志与发布在这里分岔（唯一一处）
    ///
    /// `seq` 分配与 `publish_frame` **逐帧无例外**——它们是协议与数据面。
    /// 只有**日志**分两级：**骨架**（[`FrameLogLevel::Skeleton`]）各自留一行进 `INFO`；
    /// **细节**（[`FrameLogLevel::Detail`]）进 `DEBUG`，其中的纯增量帧再被
    /// [`DeltaLogCoalescer`] 折成一行为统计（`[T#4..583] id - Update 580 帧 / +1234c`）。
    /// 分级与折行前后的可观测差异**只有 stderr 的行数**（不变量 #28）。
    ///
    /// 被并入的纯增量帧其 `detail` 不被读取（调用方传空串即可）。
    fn emit(&mut self, message: cm::ChatMessage, log: FrameLog) {
        self.seq += 1;
        let seq = self.seq;

        // 折行器先吃这一帧：可能冲刷出上一段纯增量的统计行。
        // **折行只对细节开放**——骨架帧（出现 / 终态 / 等待用户 / 删除）必须各自留行，
        // 它们正是折行要保住的东西。
        let (flushed, absorbed) =
            self.delta_log
                .feed(&message, seq, log.level == FrameLogLevel::Detail);
        if let Some(line) = flushed {
            plugin_debug!("session", "{line}");
        }
        // 被并入的纯增量帧不产生自己的行；其余帧照常打印（带 status 列与 detail）。
        if !absorbed {
            let status = message.status.as_ref().map(|s| s.as_str()).unwrap_or("-");
            let line = render_frame_line(seq, &message.id, status, &log.detail);
            match log.level {
                FrameLogLevel::Skeleton => plugin_info!("session", "{line}"),
                FrameLogLevel::Detail => plugin_debug!("session", "{line}"),
            }
        }

        publish_frame(&NodeEvent {
            session_id: self.session_id.clone(),
            seq,
            message,
        });
    }
}

#[cfg(test)]
#[path = "transcript.test.rs"]
mod tests;
