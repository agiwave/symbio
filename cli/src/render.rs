//! 会话转写的终端渲染。
//!
//! ## 输出通道约定
//!
//! - **stdout**：只放模型产出的正文（非交互模式下可直接 `| 下游程序`）。
//! - **stderr**：进度、工具调用、错误、日志 —— 可整体 `2>/dev/null` 静音。
//!
//! ## 输入是**消息载荷**
//!
//! 渲染器直接消费 VDFS 变更里的 [`ChatMessage`] 载荷（`vdfs` 频道、落点 =
//! 消息目录 `<sid>/message`，见 `client.rs`）。一帧就是一条消息，语义全在字段上：
//!
//! | 帧里有什么 | 渲染动作 |
//! | --- | --- |
//! | `delta` | 身份/状态合并进本地快照；正文累加并**直接输出**（流式热路径，O(delta)） |
//! | `content` | 整条替换本地快照；只把**比已输出更长的尾部**打出来（重写无法回收，留痕） |
//! | `status = removed` | 删掉本地快照（工具恢复会先删旧子节点再重建） |
//!
//! ## 为什么仍然要维护一份本地快照
//!
//! `delta` 把增量累积进快照；身份字段（`msg_type` / `role` / `name`）决定「这帧
//! 该不该回显」。这层合并只依赖帧自身与本地快照——不依赖任何 VDFS 回读，
//! 也不需要旧协议那种「全量帧差分」的居中层：`delta` 本身就是要打印的新增正文。
//!
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use symbio::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use symbio::symbio_core::vdfs_provider::VDFS_STATUS_WORKING;

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

    /// 一条消息帧：身份/状态合并进本地快照，正文按**字段语义**落地后回显。
    ///
    /// - `delta` = 追加：这一段就是新到达的正文，直接打印（流式热路径）；
    /// - `content` = 整条替换：只打印「比已输出更长的尾部」——终端的已输出内容
    ///   收不回来，正文被改写时留痕而不是假装没发生过；
    /// - `status = removed` = 删除：丢本地快照。
    ///
    /// 是否回显由 [`Self::render`] 按 `msg_type` / `role` 判定（工具结果与参数
    /// 都不回显）。目标未知时就地建占位快照（帧自给自足）。
    pub fn on_message(&mut self, message: &ChatMessage) {
        if message.id.is_empty() {
            return;
        }
        // 删除帧：本 CLI 不做消息树持久渲染，丢快照即可。
        if message.status == Some(MessageStatus::Removed) {
            self.msgs.remove(&message.id);
            return;
        }

        let mut rewritten_to = None;
        let (delta, mtype, role, failed, error, name) = {
            let merged = self.msgs.entry(message.id.clone()).or_default();
            // 身份字段：有则覆盖（首帧建立身份；重复携带以最后到达为准）。
            if message.parent_id.is_some() {
                merged.parent_id = message.parent_id.clone();
            }
            if message.role.is_some() {
                merged.role = message.role.clone();
            }
            if message.msg_type.is_some() {
                merged.msg_type = message.msg_type.clone();
            }
            if message.name.is_some() {
                merged.name = message.name.clone();
            }
            if message.tool_call_id.is_some() {
                merged.tool_call_id = message.tool_call_id.clone();
            }
            if message.meta.is_some() {
                merged.meta = message.meta.clone();
            }
            if message.seq.is_some() {
                merged.seq = message.seq;
            }
            if message.timestamp.is_some() {
                merged.timestamp = message.timestamp;
            }
            if message.status.is_some() {
                merged.status = message.status.clone();
            }
            if message.error.is_some() {
                merged.error = message.error.clone();
            }

            let mut delta = String::new();
            if let Some(d) = &message.delta {
                // 增量：追加到快照尾部，并原样打印
                match merged.content.as_mut() {
                    Some(MessageContent::Text(buf)) => buf.push_str(d),
                    _ => merged.content = Some(MessageContent::Text(d.clone())),
                }
                delta = d.clone();
            } else if let Some(content) = &message.content {
                // 完整正文：整条替换快照；只有"比已输出更长"的部分能打印
                let prev = merged
                    .content
                    .as_ref()
                    .map(MessageContent::to_text)
                    .unwrap_or_default();
                let next = content.to_text();
                delta = next.strip_prefix(&prev).unwrap_or_default().to_string();
                if delta.is_empty() && next != prev {
                    rewritten_to = Some(next.clone());
                }
                merged.content = Some(content.clone());
            }
            (
                delta,
                merged.msg_type.clone().unwrap_or_default(),
                merged.role.clone(),
                merged.status == Some(MessageStatus::Failed),
                merged.error.clone(),
                merged.name.clone(),
            )
        };

        self.render(&message.id, mtype, role, delta, name);
        if let Some(next) = rewritten_to {
            self.warn(&format!(
                "⚠ 正文被重写（{} 字符），终端已输出的部分无法回收",
                next.chars().count()
            ));
        }
        self.announce_failure(&message.id, failed, error);
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
    /// 用户看到的失败已经在 [`Self::on_message`]（消息级）与调用方的结局判定里报过。
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
