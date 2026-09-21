//! 会话转写的终端渲染。
//!
//! ## 输出通道约定
//!
//! - **stdout**：只放模型产出的正文（非交互模式下可直接 `| 下游程序`）。
//! - **stderr**：进度、工具调用、错误、日志 —— 可整体 `2>/dev/null` 静音。
//!
//! ## 输入是显式操作，不再是增量补丁
//!
//! 渲染器直接消费后端的 [`NodeOp`]（经 `session/stream` 下发，见
//! `symbio_core::transcript_stream`）：
//!
//! | 操作 | 载荷 | 渲染动作 |
//! | --- | --- | --- |
//! | `upsert` | **完整消息快照** | 整条替换本地快照，只输出比已打印部分多出来的正文 |
//! | `append` | 仅 `delta` | 追加进本地快照并**直接输出**（流式热路径，O(delta)） |
//! | `remove` | `message_id` | 删掉本地快照（工具恢复会先删旧子节点再重建） |
//! | `reset` | — | 本地快照作废（已打印的正文不可回收） |
//! | `warn` | — | 会话级状态，经**会话节点**下发，不在此处理 |
//!
//! ## 为什么仍然要维护一份本地快照
//!
//! `append` 把增量累积进快照，随后收尾的 `upsert`（全量）与快照做差分，才能
//! 算出「还没打印过的那一段」。这层差分是终端增量输出必需的，但它只依赖
//! **同一条消息的先后两帧**——不再依赖任何 VDFS 回读，也不再需要
//! `TranscriptPatchBuilder` 那种「把全量变更折算成补丁」的居中层。

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use symbio::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use symbio::symbio_core::vdfs_provider::VDFS_STATUS_WORKING;

fn text_of(m: &ChatMessage) -> String {
    m.content
        .as_ref()
        .map(MessageContent::to_text)
        .unwrap_or_default()
}

fn write_stdout(s: &str) {
    let mut out = io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

fn write_stderr(s: &str) {
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "{s}");
}

pub struct Renderer {
    quiet: bool,
    verbose: bool,
    /// 本轮已合并的消息快照（id → 完整消息），用于算「还没打印过的那一段」
    msgs: HashMap<String, ChatMessage>,
    /// 已播报过的工具调用 id
    tools_announced: HashSet<String>,
    /// 已播报过失败原因的消息 id
    failures_announced: HashSet<String>,
    /// 本轮是否已经输出过正文（用于收尾换行判断）
    pub wrote_text: bool,
}

impl Renderer {
    pub fn new(quiet: bool, verbose: bool) -> Self {
        Self {
            quiet,
            verbose,
            msgs: HashMap::new(),
            tools_announced: HashSet::new(),
            failures_announced: HashSet::new(),
            wrote_text: false,
        }
    }

    /// 开始新一轮：丢弃上一轮的快照（快照只在单轮内做增量合并）。
    pub fn begin_turn(&mut self) {
        self.msgs.clear();
        self.tools_announced.clear();
        self.failures_announced.clear();
        self.wrote_text = false;
    }

    /// 一轮收尾：若输出过正文则补一个换行，让下一个提示符另起一行。
    pub fn end_turn(&mut self) {
        if self.wrote_text {
            write_stdout("\n");
        }
    }

    /// 完整快照（`NodeOp::Upsert`）：整条替换本地快照，只输出新增正文。
    pub fn on_upsert(&mut self, snapshot: &ChatMessage) {
        if snapshot.id.is_empty() {
            return;
        }

        // 先合并再渲染：解构出决策所需的全部信息，尽早结束对 self.msgs 的借用。
        let (delta, mtype, role, failed, error, name) = {
            let merged = self.msgs.entry(snapshot.id.clone()).or_default();
            let before = text_of(merged);
            *merged = snapshot.clone();
            let after = text_of(merged);
            // 正常路径下 `append` 已把增量累积进快照，收尾快照与之相同 → delta 为空。
            // 后端若直接给全量（未经 append），strip_prefix 失败即整段输出，不会丢字。
            let delta = match after.strip_prefix(before.as_str()) {
                Some(rest) => rest.to_string(),
                None => after,
            };
            (
                delta,
                merged.msg_type.clone().unwrap_or_default(),
                merged.role.clone(),
                merged.status == Some(MessageStatus::Failed),
                merged.error.clone(),
                merged.name.clone(),
            )
        };

        self.render(&snapshot.id, mtype, role, delta, name);
        self.announce_failure(&snapshot.id, failed, error);
    }

    /// 流式增量（`NodeOp::Append`）：累积进本地快照并直接输出。
    ///
    /// **追加的目标不止正文**：工具参数（与 Text / Reasoning 同构）与工具响应
    /// （`tool_executor` 透传子会话的 `Append`）同样走这里。因此不能假定"增量即正文"
    /// ——是否输出由 [`Self::render`] 按 `msg_type` / `role` 判定（工具结果与参数
    /// 都不回显）。目标未知（连 `upsert` 都没收到过）时就地建快照，避免漏字。
    pub fn on_append(&mut self, message_id: &str, delta: &str) {
        if message_id.is_empty() || delta.is_empty() {
            return;
        }
        let (mtype, role) = {
            let merged = self.msgs.entry(message_id.to_string()).or_default();
            match merged.content.as_mut() {
                Some(MessageContent::Text(buf)) => buf.push_str(delta),
                _ => merged.content = Some(MessageContent::Text(delta.to_string())),
            }
            (
                merged.msg_type.clone().unwrap_or_default(),
                merged.role.clone(),
            )
        };

        self.render(message_id, mtype, role, delta.to_string(), None);
    }

    /// 后端精确删除某条消息（工具恢复时清理旧子节点）——本 CLI 不做消息树持久渲染，只丢快照。
    pub fn on_remove(&mut self, message_id: &str) {
        self.msgs.remove(message_id);
    }

    /// 正文与工具播报的统一呈现：按 `msg_type` / `role` 决定回显什么。
    ///
    /// 失败提示不在这一层——那是消息级终态的一部分，与"正文是否该回显"没有
    /// 共同依据，见 [`Self::announce_failure`]。
    fn render(
        &mut self,
        id: &str,
        mtype: MessageType,
        role: Option<MessageRole>,
        delta: String,
        name: Option<String>,
    ) {
        match mtype {
            MessageType::Text => {
                // 只回显模型正文：用户消息由 CLI 自己输出，工具结果文本不回显
                // （role 尚未到达的早期增量按"助手"处理，避免漏字）。
                let suppress = matches!(
                    role,
                    Some(MessageRole::User) | Some(MessageRole::Tool) | Some(MessageRole::System)
                );
                if !suppress && !delta.is_empty() {
                    write_stdout(&delta);
                    self.wrote_text = true;
                }
            }
            MessageType::Reasoning => {
                if self.verbose && !delta.is_empty() && !self.quiet {
                    // 推理内容走 stderr，避免污染 stdout 的正文流
                    write_stderr(&format!("\x1b[2m{delta}\x1b[0m"));
                }
            }
            MessageType::ToolCall => {
                if self.tools_announced.insert(id.to_string()) {
                    let name = name.unwrap_or_else(|| "tool".to_string());
                    if !self.quiet {
                        if self.wrote_text {
                            write_stdout("\n");
                            self.wrote_text = false;
                        }
                        write_stderr(&format!("⚙ 调用工具: {name} …"));
                    }
                }
            }
            // Turn（组合节点）/ UserPrompt（需审批的交互卡）：无正文可渲染。
            _ => {}
        }
    }

    /// 失败提示（消息级终态）：一条消息只播报一次，且先给已输出的正文补一个换行。
    ///
    /// `failed` 与 `error` 分开传是照搬调用方的判别（状态为 `Failed` 且带原因）：
    /// 两者缺一都不播报——状态是 `Failed` 却没有原因的消息，给不出可行动的信息，
    /// 占一行只是噪声。
    fn announce_failure(&mut self, id: &str, failed: bool, error: Option<String>) {
        if !failed {
            return;
        }
        if let Some(err) = error.filter(|e| !e.is_empty()) {
            if self.failures_announced.insert(id.to_string()) {
                if self.wrote_text {
                    write_stdout("\n");
                    self.wrote_text = false;
                }
                write_stderr(&format!("✖ 失败: {err}"));
            }
        }
    }

    /// 会话运行态提示。
    ///
    /// 入参是**会话节点的状态词**（`VDFS_STATUS_*`），由会话节点承载（VDFS watch 域），
    /// 不再是旧事件频道的 `busy` / `idle` —— 旧的 `other => "· 状态: …"` 兜底也一并
    /// 删了：节点状态词是闭集，能走到那里的只有未知词，而它不值得占一行——真正需要
    /// 用户看到的失败已经在 [`Self::on_upsert`]（消息级）与调用方的结局判定里报过。
    ///
    /// **不去重**：「只在新状态上报一次」只有调用方做得到（它是唯一能看到上一帧的
    /// 地方），重复的 `working` 帧在这层去不掉——这正是它从前会两行“处理中”的原因。
    pub fn on_status(&mut self, status: &str) {
        if self.quiet {
            return;
        }
        if status == VDFS_STATUS_WORKING {
            write_stderr("… 处理中");
        }
    }

    pub fn on_abort(&mut self) {
        write_stderr("■ 已中止");
    }

    /// REPL 用的即时提示（不计入正文）。
    pub fn notice(&self, msg: &str) {
        if !self.quiet {
            write_stderr(msg);
        }
    }

    /// 告警行（走 stderr，静默模式下不输出）。
    ///
    /// 消费端遇到「本该能处理但缺载荷」时据此留痕：静默跳过是事故，
    /// 而把判断留在调用方就会在每个调用点各写一次 `quiet` 分支。
    pub fn warn(&self, msg: &str) {
        if !self.quiet {
            write_stderr(msg);
        }
    }
}
