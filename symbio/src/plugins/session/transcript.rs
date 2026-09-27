//! 会话转写（Transcript）：实时链路的**唯一写入点与发射器**（session 域内）。
//!
//! - **唯一写入点**：一切消息级变更（模型流式 / 工具执行 / 恢复重写 / 压缩 /
//!   用户消息定稿）都以**一条消息帧**（[`cm::ChatMessage`]）进入
//!   [`Transcript::apply`]——内存图、位置序号、核心日志、**变更投递**四件事在同一个
//!   函数里完成，没有任何旁路。
//! - **位置序号**：每条消息在 `<根>/session/<id>/message` 这个**文件夹**里的位置。
//!   在途消息此前没有号（只能靠 `timestamp` 兜底，同毫秒即并列），现在**创建时**就分配
//!   （见 [`INFLIGHT_SEQ_BASE`]）。它是**节点属性**，不是投递属性——消费端按它排序，
//!   与变更的到达顺序无关。
//! - **变更投递**：每个变更投给 `kind = "vdfs"` 的订阅表（[`VdfsChangeSubscriptions`]），
//!   落点是**那条消息节点自身的地址** `<id>/message/<mid>`，载荷就是这条 `ChatMessage`
//!   （正文增长 = `delta`、整条替换 = `content`、删除 = `status = removed`）。
//!   **语义全在字段上，没有操作枚举**——这与 `ChatMessage` 帧自己的设计同源。
//! - **核心日志**：分**骨架**与**细节**两级（见 [`FrameLogLevel`]）。骨架 = 节点
//!   出现 / 终态 / 等待用户 / 删除，进 `INFO`——"什么出现了、什么时候结束"一眼看完；
//!   细节 = `Update` 帧与纯增量的折行统计，进 `DEBUG`（`--verbose` 或
//!   `SYMBIO_LOG=debug` 放开）。细节里的**连续同节点纯增量**（帧带 `delta`、无状态
//!   迁移）再被 [`DeltaLogCoalescer`] 折成一行统计：一次流式回复有几百个增量帧，
//!   逐帧一行会把时间线淹成噪声（实测一段 200 字回复 = 580 行 `+Nc`，占该轮 stderr
//!   的 93%）。分级与折行**只作用于日志**。
//!   日志的 `[T#n]` 用的是**帧计数器**（`frame_no`，只进日志、不下发），与位置序号无关。
//! - **投递合帧**：相邻的**同节点纯增量**在 [`DELIVER_WINDOW_MS`] 的窗口内合成一帧
//!   再投（见 [`Transcript::deliver`]）。语义逐字等价（正文一个字符不多不少），
//!   但一次流式回复的出帧数降到约 1/6——那几百帧本来是显示刷新率吃不下、
//!   只有 IPC 成本没有信息量的东西。日志的折行与投递的合帧是**两件事**：
//!   前者按帧序折叠成统计行，后者按时间窗口合并载荷，触发条件不同，因此不共用一个结构。
//!
//! ## 与 VDFS 的边界
//!
//! 消息的**实时面**与**历史面**是**同一条** `vdfs/watch`：变更的落点是**那条消息
//! 节点自身的地址** `<id>/message/<mid>`（与 `list` / `read` 的节点地址逐字同源），
//! 业务载荷 `data` 就是那条 `ChatMessage`——`delta` 有 ⇒ 尾部追加（零回读）、
//! `content` 有 ⇒ 整条替换、`status = removed` ⇒ 就地移除。
//! 会话节点自身的状态（working / error / warning）也是 VDFS 变更（`<id>`，
//! `data` = 全量节点视图），与侧栏会话清单共用同一份订阅。
//!
//! 地址形状统一为 `<sid>/<集合段>/<项 id>`：会话是**容器**，其下是若干并列的
//! **集合**（消息 / 子会话 / 记忆 / 工作目录，后续还会有任务列表、请求队列……），
//! 因此「哪一类集合」由**地址段**回答，机制不认识任何一类集合——新增一类集合
//! 不需要在信封上加概念。

use super::plugin::message_path;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::{VdfsChange, VdfsChangeSubscriptions};
use crate::{plugin_debug, plugin_error, plugin_info};
use indexmap::IndexMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod frames;
mod inbox;

// 帧构造 / 日志合并 / 合帧窗口（纯机械；状态机仍在根）
mod deliver;
// 域内共享面：状态机（`impl Transcript`）与本域测试经 `super::*` 取用。
use self::deliver::{
    frame_log_of, render_frame_line, DeltaLogCoalescer, FrameLog, FrameLogLevel, PendingDelta,
    DELIVER_WINDOW_MS,
};

// 域内门面：状态帧 / 删除帧的构造与发射经 `transcript::X` 取用，调用方不感知
// 域内文件划分（域内子模块经 `use super::*` 取用门面）。
pub use self::frames::{llm_emit_removed, llm_emit_state, llm_state_frame};

/// 在途消息**位置序号**的起点。
///
/// 在途消息尚未落库，存储还没给它分配号；但并行工具的消息必须在**创建时**就有权威
/// 顺序（否则前端只能回退 `timestamp`，同毫秒即并列）。这里给在途消息一段**远离**
/// 存储序号的空间：存储分配的序号是「第几条消息」量级（很小），而
/// `plugin::nodes::ordered` 按 `seq` 升序——因此从本值
/// 开始的在途序号必然排在全部历史之后，正是「最新的消息在末尾」。
///
/// 落库后由 `overlay_live` / `persisted` 交回存储分配的序号，这段临时序号随之作废。
///
/// # 「在途号 / 存储号」是两个独立递增的计数器，共存的前提只有一条
///
/// **在途号永不落库**。一旦某个在途号被当成权威号存了下来，存储水位就被抬进在途
/// 号段，此后两个计数器在同一数值区间里各自递增、**必然撞号**——实测同一会话里
/// 「本轮用户消息」与「压缩节点」各持同一个在途号，
/// `assertTranscriptInvariants` 的「seq 严格递增」当场失败。
///
/// 因此有两半，缺一不可：
///
/// 1. **存储边界拒收在途号**（见 [`is_inflight_seq`]，落实在 `chat_session` 的两条
///    写入路径上）——这是机制：任何调用点都不必"记得"先清号，漏掉也不会再泄漏；
/// 2. **本值取 `1 << 50`**：泄漏发生过的存量会话，其存储水位停在旧的号段。若新
///    在途号仍从那一段起，它会**排在那些存量号之前**（「最新的消息在末尾」当场
///    失效）。抬高之后两段重新分离，而 `1 << 50` ≈ 1.1e15，真实序号追不上它。
pub(crate) const INFLIGHT_SEQ_BASE: i64 = 1 << 50;

/// 这个 `seq` 是否是**在途占位号**（而非存储分配的权威号）。
///
/// 判据只有一条：**号段**。存储号是「第几条消息」量级，从 1 起逐个递增；在途号从
/// [`INFLIGHT_SEQ_BASE`] 起。两者之间隔着 ~1e15 的空档，「`>= INFLIGHT_SEQ_BASE`」
/// 因此等价于「不是权威号」。
///
/// 它是**存储边界**的守卫（`chat_session::write::append_messages` / `replace_messages`）：
/// 权威号只能由存储分配，带进来的在途号一律摘掉、重新分配。这条不变式放在边界上
/// 而不是各调用点上，是因为写入路径有五条以上，而"记得清号"是典型会漏的一类约定
/// ——漏掉的后果是**静默的**：号看起来都正常，直到两个计数器撞上才暴露。
pub(crate) fn is_inflight_seq(seq: i64) -> bool {
    seq >= INFLIGHT_SEQ_BASE
}

/// 会话转写：内存图（在途视图）+ 位置序号分配 + 变更投递。
///
/// 挂在 [`crate::plugins::session::active::ActiveSessionState`] 上（每会话一个），
/// 消费循环是它唯一的常规写入者；落库回包（用户消息 / 助手侧增量 / 压缩节点，
/// 见 `docs/vdfs-session-messages.md` §3.4）与压缩发射器经同一入口写入。
pub struct Transcript {
    session_id: String,
    /// 下一个要分配的**在途位置序号**（见 [`INFLIGHT_SEQ_BASE`]）。
    /// 只在**首次见到**一条消息、且该帧没带存储分配的号时消耗。
    next_seq: i64,
    /// **只进日志**的帧计数器——不是协议的一部分，也不下发。
    /// 日志的 `[T#n]` 用它，因此「一次流式回复有几百帧」在时间线上仍然可数
    /// （位置序号对同一节点的每一帧都是同一个值，用它就失去这个信息）。
    frame_no: u64,
    nodes: IndexMap<String, cm::ChatMessage>,
    /// 变更投递表——**必须是 session provider 的那一份**（不是全局 `hub_of`）：
    /// `vdfs/watch` 登记的是那张表，投到别处等于没人收到。
    changes: Arc<VdfsChangeSubscriptions>,
    /// 待投递的**纯增量窗口**（见 [`Self::deliver`]）。
    pending: Option<PendingDelta>,
    /// 日志合并器（**只影响日志**，不参与序号与投递）。
    delta_log: DeltaLogCoalescer,
}

impl Transcript {
    /// 构造一份会话转写。
    ///
    /// `changes` 必须是 **session provider 自持的那一张表**（插件的
    /// `change_subs`）：`vdfs/watch` 登记的就是它，投到全局 `hub_of(kind)` 上等于
    /// 没人收到——那张表上没有本 provider 的订阅者。
    pub fn new(session_id: String, changes: Arc<VdfsChangeSubscriptions>) -> Self {
        Self {
            session_id,
            next_seq: INFLIGHT_SEQ_BASE,
            frame_no: 0,
            nodes: IndexMap::new(),
            changes,
            pending: None,
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
    /// | 身份字段 / `meta` / `timestamp` | 有则覆盖（发射端持有当前完整值） |
    /// | `seq` | 帧带了就用（存储权威值）；**没带且是新节点** ⇒ 就地分配在途号（[`INFLIGHT_SEQ_BASE`]） |
    ///
    /// 未知 id 用帧内信息建占位再合并——帧自给自足，不依赖任何先行帧（含它的顺序：
    /// 在途号在这里就有，不必等落库）。
    ///
    /// 这里分配的**在途号是节点属性，不是落库值**：它只用来在图上排序，直到
    /// [`Self::persisted`] 把该节点交回存储。把它带进存储的路径一律被存储边界拒收
    /// （见 [`INFLIGHT_SEQ_BASE`] 的说明——那里记着一次真实的撞号事故）。
    ///
    /// ## 协议违例：同帧既带增量又带完整正文
    ///
    /// 该拼接还是该替换？语义不可判定，报错丢弃：不发布、不占在途号——不让一个
    /// 语义不可判定的帧进内存图，也不让它上实时面。
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

        // 删除：不落图，就地广播该状态帧（帧照常占日志号，「删了什么」在时间线上可追溯）。
        // 删除的线上表达就是这条带 `removed` 状态的帧本身（消息词汇本就有这个状态）。
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
        // 位置序号：**帧带就用**（存储权威值，落库后的对齐帧），**没带且是新节点**
        // 就本轮分配一个在途号（见 `INFLIGHT_SEQ_BASE`）。先算再入图——下面那段要
        // 独占 `self.nodes` 的可变借用。
        let allocated = match msg.seq {
            Some(s) => Some(s),
            None if !existed => {
                let s = self.next_seq;
                self.next_seq += 1;
                Some(s)
            }
            None => None,
        };
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
        // 存储侧权威值（落库后回发的对齐帧）；新节点则落在上面分配的在途号上。
        if let Some(s) = allocated {
            node.seq = Some(s);
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
        // 信封没有操作枚举：「首次出现还是更新」不单独成字段——帧自给自足
        // （身份 / 正文 / 状态都在 `data` 里），消费端按字段落地，不需要分派键。
        //
        // **首帧发全量**：未知 id 上的窄增量（`{id, delta}`）会让消费端拿不到正文
        // 基线——而图里的节点此刻已经合并完毕（身份 / 正文 / 在途号都在），把这份
        // 自给自足的副本发出去，消费端零回读；既有节点照原帧发（窄增量保持窄，
        // 那是流式正文的主干道，逐帧克隆全量纯属浪费）。
        let frame = if existed { msg } else { node.clone() };
        self.emit(frame, FrameLog { level, detail });
    }

    /// 落库回执：权威副本已写入存储，把节点从内存图移除（不发布）。
    ///
    /// 存储是唯一权威；图里只留**在途**节点（VDFS 转写列表的叠加来源）。
    pub fn persisted(&mut self, ids: &[String]) {
        // 落库 = 该节点的增长段结束：把待合并的增量 run 与待投递的增量窗口都收尾。
        if let Some(line) = self.delta_log.flush() {
            plugin_debug!("session", "{line}");
        }
        self.flush_pending();
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
    /// 顺带冲刷待合并的增量 run 与待投递的增量窗口——轮次边界是"这一段增长结束了"
    /// 的最强信号，也是最后一段增量不至于被吞掉的保证（`clear` 在轮次起止各调一次）。
    pub fn clear(&mut self) {
        if let Some(line) = self.delta_log.flush() {
            plugin_debug!("session", "{line}");
        }
        self.flush_pending();
        self.nodes.clear();
    }

    /// 打一行核心日志、投递一条 VDFS 变更。`frame_no` 在每一帧上 +1。
    ///
    /// ## 这里分岔成两件事（唯一一处）
    ///
    /// | 出口 | 规则 |
    /// |---|---|
    /// | 日志 | 分两级：**骨架**（[`FrameLogLevel::Skeleton`]）各自留一行进 `INFO`；**细节**（[`FrameLogLevel::Detail`]）进 `DEBUG`，其中纯增量帧再被 [`DeltaLogCoalescer`] 折成一行统计（`[T#4..583] id - Update 580 帧 / +1234c`） |
    /// | 投递 | 纯增量进**合帧窗口**（[`Self::deliver`]），其余立刻发 |
    ///
    /// 被并入日志的纯增量帧其 `detail` 不被读取（调用方传空串即可）。
    ///
    /// ## 投递的形状：`<sid>/message/<mid>` 上 `data = ChatMessage`
    ///
    /// 信封的 `path` 恒为**被变更节点自身的地址**——消息就是 `<sid>/message/<mid>`
    /// 这个文件（与 `list` / `read` 返回的节点地址、与文档里的地址模型逐字同源）。
    /// 会话是**容器**，其下是若干**并列的集合**（消息 / 子会话 / 记忆 / 工作目录，
    /// 后续还会有任务列表、请求队列……），因此地址形状统一为
    /// `<sid>/<集合段>/<项 id>`——机制不认识任何一类集合，新增一类集合不需要
    /// 在信封上新增概念，也不需要消费端学一条新的「身份在哪」的规则。
    ///
    /// `data` 就是**帧本身**（与内存图收到的同一条 `ChatMessage`，其 `id` 与路径
    /// 末段是同一个身份）：`delta` 有 ⇒ 尾部追加、`content` 有 ⇒ 整条替换、
    /// `status = removed` ⇒ 就地移除——消费端按字段落地，零翻译。增量帧只带
    /// `delta`（窄载荷），全量帧带身份 + 正文；首帧（无论上游发的是窄增量还是全量）
    /// 发图里合并后的**全量副本**，让消费端零回读即得基线。同帧 `delta` + `content`
    /// 已在 [`Self::apply`] 入口拒绝。
    fn emit(&mut self, message: cm::ChatMessage, log: FrameLog) {
        self.frame_no += 1;
        let frame_no = self.frame_no;

        // 折行器先吃这一帧：可能冲刷出上一段纯增量的统计行。
        // **折行只对细节开放**——骨架帧（出现 / 终态 / 等待用户 / 删除）必须各自留行，
        // 它们正是折行要保住的东西。
        let (flushed, absorbed) =
            self.delta_log
                .feed(&message, frame_no, log.level == FrameLogLevel::Detail);
        if let Some(line) = flushed {
            plugin_debug!("session", "{line}");
        }
        // 被并入的纯增量帧不产生自己的行；其余帧照常打印（带 status 列与 detail）。
        if !absorbed {
            let status = message.status.as_ref().map(|s| s.as_str()).unwrap_or("-");
            let line = render_frame_line(frame_no, &message.id, status, &log.detail);
            match log.level {
                FrameLogLevel::Skeleton => plugin_info!("session", "{line}"),
                FrameLogLevel::Detail => plugin_debug!("session", "{line}"),
            }
        }

        self.deliver(message);
    }

    /// 投递一帧：**相邻的同节点纯增量在 [`DELIVER_WINDOW_MS`] 内合成一帧**。
    ///
    /// ## 为什么可以合
    ///
    /// 信封没有操作枚举：一帧增量的语义就是「这条节点的正文尾部追加了这些字符」，
    /// 消费端按字段落地。两帧相邻同节点的纯增量（`delta` 有、`content` 与 `status`
    /// 都无）合并成一帧后**语义逐字等价**——正文一个字符不多不少。顺序也不是风险：
    /// `ChatMessage.seq` 是**节点属性**（不是投递属性），到达顺序本来就不参与排序。
    ///
    /// ## 为什么要合
    ///
    /// 一次流式回复的帧数由模型决定（实测一段 200 字回复 ≈ 580 帧），而每帧的成本
    /// 与帧数成正比：遍历订阅表、克隆整帧、序列化成信封、跨 JS 桥、前端反序列化。
    /// 60Hz 的显示刷新率本身就吃不下逐帧投递——多出来的帧不产生任何用户可见的
    /// 信息，只把 IPC 打满。窗口取值的理由见 [`DELIVER_WINDOW_MS`]。
    ///
    /// ## 什么形状**必然**立刻发（不进窗口）
    ///
    /// 首帧（图里合并后的**全量副本**，消费端的基线）、状态迁移、删除、整条替换
    /// ——它们都带 `content` 或 `status`，天然不满足「纯增量」。**换节点**（`id`
    /// 不同）也立刻冲刷：合并只在同一个文件的一段连续增长内部发生。
    ///
    /// ## 窗口过期不需要后台任务
    ///
    /// 每收到一帧先看窗口是否已过 [`DELIVER_WINDOW_MS`]，过了就冲刷。因此最坏情形是
    /// 「一段增长的最后一帧晚于窗口到达」——而一段增长的两个端点（首帧与终态帧）
    /// 都不进窗口，所以没有内容会被无限期扣住。
    ///
    /// ## 顺序不能反：非纯增量帧必须先冲刷再发
    ///
    /// 待投递的增量是**更早**的正文。若先发终态帧、后发增量，消费端会看到
    /// 「这条消息已结束」之后正文又长了一截（前端据此判定的运行态已经收敛）。
    fn deliver(&mut self, mut message: cm::ChatMessage) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.started.elapsed() >= Duration::from_millis(DELIVER_WINDOW_MS))
        {
            self.flush_pending();
        }

        // 「纯增量」= 可合帧的唯一形状（其余立刻发，见上方表）
        let delta = if message.content.is_none() && message.status.is_none() {
            message.delta.take()
        } else {
            None
        };
        let Some(delta) = delta else {
            self.flush_pending();
            self.publish(message);
            return;
        };

        // 同窗口 + 同节点 ⇒ 并入（正文等价）；否则冲刷旧窗口，为新节点开窗口。
        // 待投递帧恒为纯增量形状（只有本函数写 `pending`，而它只收纯增量）。
        if let Some(buf) = self
            .pending
            .as_mut()
            .filter(|p| p.frame.id == message.id)
            .and_then(|p| p.frame.delta.as_mut())
        {
            buf.push_str(&delta);
            return;
        }
        self.flush_pending();
        message.delta = Some(delta);
        self.pending = Some(PendingDelta {
            frame: message,
            started: Instant::now(),
        });
    }

    /// 冲刷待投递的增量窗口（`persisted` / `clear` / `emit_session_state` 与
    /// 非纯增量帧都经它，保证「更早的正文先出」）。
    fn flush_pending(&mut self) {
        let Some(p) = self.pending.take() else {
            return;
        };
        self.publish(p.frame);
    }

    /// 变更投递的**唯一出口**（`changes.notify` 只在这里被调用）。
    fn publish(&self, message: cm::ChatMessage) {
        self.changes.notify(&VdfsChange::with_data(
            message_path(&self.session_id, &message.id),
            &message,
        ));
    }

    /// 发布一次**会话运行态**变更：`<sid>` 上 `data = VdfsNode`（全量节点视图）。
    ///
    /// 与消息走**同一张订阅表**：会话节点就是 `<sid>`（它既是清单里的文件又是
    /// 容器目录，变更落点是它自身）。这与 ADR-025 的结论一致——实时面与历史面
    /// 是同一条 `vdfs/watch`。
    ///
    /// ## `data` 为什么带**全量节点视图**
    ///
    /// 视图由编排层在**调用本函数那一刻**从权威源（`list` / `stat` 的同一构造点
    /// `session_node`）取好——它不是缓存的旧副本，而是「此刻的状态」。消费端
    /// **零回读**就地收敛：状态迁移是最需要即时的路径，一次状态迁移一次 IPC
    /// 恰恰是最不该省的那一步。丢失不要紧（状态是幂等的，下一次 `list` 收敛）。
    /// `None`（会话已删、取不到视图）⇒ 退化为无载荷变更：回读 `NotFound` 即删除。
    ///
    /// 调用时机由编排层保证：正常收尾在「清在途 → 复位 `is_working`」之后、
    /// 中止收尾在 `converge_inflight` 之后——两者都在本轮**最后一条**消息帧之后。
    ///
    /// ## 必须先冲刷待投递的增量
    ///
    /// 它是本轮**更早**的正文，而本条变更说的是「这一轮结束了」。顺序反了，
    /// 前端会先收敛为已空闲、再补上一截正文（`sessionNodeSync` 在
    /// `working → 非 working` 迁移时就会清掉活动角标，节点却还在长）。
    pub fn emit_session_state(&mut self, node: Option<crate::symbio_core::VdfsNode>) {
        self.frame_no += 1;
        self.flush_pending();
        match &node {
            Some(n) => plugin_info!(
                "session",
                "[T#{}] <session> {} - 会话运行态 {}",
                self.frame_no,
                self.session_id,
                n.status
            ),
            None => plugin_info!(
                "session",
                "[T#{}] <session> {} - 会话运行态变更（无视图，回读收敛）",
                self.frame_no,
                self.session_id
            ),
        }
        // 视图在手 ⇒ `data = VdfsNode`（消费端零回读）；视图缺席（会话已删）⇒
        // 无载荷变更，消费端回读 `stat` 得 `NotFound` 即自然收敛。
        let change = match node {
            Some(n) => VdfsChange::with_data(&self.session_id, &n),
            None => VdfsChange::bare(&self.session_id),
        };
        self.changes.notify(&change);
    }
}

#[cfg(test)]
#[path = "transcript.test.rs"]
mod tests;
