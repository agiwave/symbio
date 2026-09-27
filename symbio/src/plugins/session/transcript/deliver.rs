//! 投递侧的**帧构造 / 日志合并 / 合帧窗口**机器
//!
//! 本文件只放「怎么把一条变更变成一帧、以及要不要合、日志打几行」这类
//! **纯机械**：类型与纯函数不持有 `Transcript` 状态；状态机（`apply` / `emit` /
//! `deliver` …）仍在 `transcript.rs`——因为它们的每一步都要读写整张内存图。
//!
//! ## 为什么两套「合帧」住在一起、而「位置序号」分家
//!
//! 本域有两套**互不相干**的合并：日志侧按**帧序**折叠同节点纯增量
//! （[`DeltaLogCoalescer`]，保住"一次增长有多少帧"这个过程量），投递侧按**时间窗口**
//! 合并载荷（[`DELIVER_WINDOW_MS`] + [`PendingDelta`]，保住"正文一个字符不少"）。
//! 两者要保住的东西不同，但都只关心**一帧怎么发**——所以同住本文件，
//! 彼此的对照关系（谁折帧序、谁折时间）一眼可见。
//!
//! 「在途位置序号」（`INFLIGHT_SEQ_BASE` / `is_inflight_seq`）则留在 `transcript.rs`：
//! 它管的是**顺序**，不是**帧**，且被 `chat_session` 的存储边界引用。
//!
//! ## 测试在哪
//!
//! 经**域测试文件** `transcript.test.rs` 覆盖（合帧 / 折行 / 帧日志的用例在那里）：
//! 这些行为只在 `Transcript::apply` 之后才可观察，脱离状态机单独测等于把
//! 同一段语义钉两遍（见 `docs/module-layout.md` §2）。

use super::*;

/// 投递合帧窗口（毫秒）——相邻的**同节点纯增量**在此窗口内合成一帧（见
/// [`Transcript::deliver`]）。
///
/// 取 50ms ⇒ 稳态约 20 帧/秒。判据是**感知阈值**而不是链路能力：流式文本在
/// 20 次/秒以上的更新率下已经看不出分块，而模型侧的帧率是它的几十倍
/// （实测一段 200 字回复 ≈ 580 帧）。比它更短的窗口省不下多少帧，更长则会
/// 让「打字机」变顿。前端另有 48ms 的落地窗口（`sessionTranscriptSync`），
/// 两者叠加后一次回复的后端 IPC 帧数降到约 1/6。
pub(crate) const DELIVER_WINDOW_MS: u64 = 50;

/// 一个正在累积的**投递窗口**：窗口内若干个同节点纯增量已被并进 `frame.delta`。
///
/// `frame` 的线上形状与单帧**逐字一致**（`id` + `delta`），消费端无从、也无需
/// 知道它被合过——合帧是投递层的优化，不是协议的一部分。
pub(super) struct PendingDelta {
    pub(super) frame: cm::ChatMessage,
    /// 窗口起点（本窗口第一帧到达的时刻）。窗口过期由**每一帧到达时**检查，
    /// 因此不需要后台定时器。
    pub(super) started: Instant,
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
/// 本结构体只被 `Transcript::emit` 用于"要不要打这一行、打成什么样"——它不认识
/// 载荷，也不参与投递。**投递侧的合帧是另一个机制**（[`Transcript::deliver`]，
/// 按时间窗口合并载荷）：两者要保住的东西不同——日志要保住"一次增长有多少帧、
/// 多少字符"这个**过程量**（所以按帧序折叠、并记录首末帧号），投递要保住
/// "正文一个字符不少"这个**正确性**（所以按时间窗口合并、与帧序无关）。
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
///
/// 字段叫 `frame_*` 而不是 `seq_*`：本结构只服务于日志，而 `seq` 这个词在本模块里
/// 已经被 [`INFLIGHT_SEQ_BASE`] 那一条语义（**位置序号**）占住了。叫错一次，
/// 下一个读代码的人就会以为折行碰了位置序号——它碰的从来只是日志行。
#[derive(Debug, PartialEq, Eq)]
struct DeltaRun {
    message_id: String,
    first_frame: u64,
    last_frame: u64,
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
        frame_no: u64,
        foldable: bool,
    ) -> (Option<String>, bool) {
        let chars = match (&msg.delta, &msg.status) {
            (Some(d), None) if foldable => d.chars().count(),
            _ => return (self.flush(), false),
        };
        if let Some(run) = self.run.as_mut() {
            if run.message_id == msg.id {
                run.last_frame = frame_no;
                run.frames += 1;
                run.chars += chars;
                return (None, true);
            }
        }
        // 换了 id：先冲刷旧 run，再为本帧开新 run。
        let flushed = self.flush();
        self.run = Some(DeltaRun {
            message_id: msg.id.clone(),
            first_frame: frame_no,
            last_frame: frame_no,
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
                self.first_frame, self.message_id, self.chars
            )
        } else {
            format!(
                "[T#{}..{}] {} - Update {} 帧 / +{}c",
                self.first_frame, self.last_frame, self.message_id, self.frames, self.chars
            )
        }
    }
}

/// 核心日志的两级：**骨架**（`INFO`）与**细节**（`DEBUG`）。
///
/// 一条时间线只有骨架值得常看；其余都是"怎么长起来的"过程量，需要时再放开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameLogLevel {
    /// **关键节点**：节点出现 / 终态 / 等待用户 / 删除。进 `INFO`，且**不可折行**。
    Skeleton,
    /// **过程细节**：`Update` 帧（`pending → streaming`、正文替换、仅 `meta` 变更）与
    /// 纯增量的折行统计。进 `DEBUG`；其中的纯增量帧可被折行器并入。
    Detail,
}

/// 一帧的日志规格（`apply` 判好、`emit` 执行——`emit` 不再反推语义）。
pub(super) struct FrameLog {
    pub(super) level: FrameLogLevel,
    /// 一行的正文描述：`<相位> [<正文形态>]`；可能被折行的帧传空串（不被读取）。
    pub(super) detail: String,
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
pub(super) fn frame_log_of(
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

/// 渲染一行核心日志：`[T#<frame>] <id> <status> [<detail>]`。
///
/// 无 `detail` 时不落尾随空格——日志要能直接复制粘贴、能整齐对齐。
pub(super) fn render_frame_line(frame_no: u64, id: &str, status: &str, detail: &str) -> String {
    if detail.is_empty() {
        format!("[T#{frame_no}] {id} {status}")
    } else {
        format!("[T#{frame_no}] {id} {status} {detail}")
    }
}
