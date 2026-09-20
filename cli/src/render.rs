//! 会话事件的终端渲染。
//!
//! ## 输出通道约定
//!
//! - **stdout**：只放模型产出的正文（非交互模式下可直接 `| 下游程序`）。
//! - **stderr**：进度、工具调用、错误、日志 —— 可整体 `2>/dev/null` 静音。
//!
//! ## 为什么本地要维护一份消息快照
//!
//! [`Renderer::on_update`] 接的是**增量补丁**（见 `ChatMessage::apply_patch`）：
//! Text / Reasoning 的 `content` 是「多了什么」，需累加；`Turn` / `ToolCall`
//! 则是全量替换。因此渲染必须先把补丁合并成完整消息，再据合并结果决定怎么显示。
//!
//! 补丁的来源是 **VDFS 变更**：`created` / `updated` 的载荷是**全量**，由
//! `symbio_core::schemas::session::transcript::TranscriptPatchBuilder` 居中
//! 折算成增量后才到这里（前端 `services/vdfsTranscriptSync.ts` 是同一份语义的
//! 另一实现），保证两端显示一致。

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
    /// 本轮已合并的消息快照（id → 完整消息）
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

    /// 开始新一轮：丢弃上一轮的快照（seq/内容只在单轮内做增量合并）。
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

    pub fn on_update(&mut self, patch: &ChatMessage) {
        if patch.id.is_empty() {
            return;
        }

        // 先合并再渲染：解构出决策所需的全部信息，尽早结束对 self.msgs 的借用。
        let (delta, mtype, role, failed, error, name) = {
            let merged = self.msgs.entry(patch.id.clone()).or_default();
            let before = text_of(merged);
            merged.apply_patch(patch);
            let after = text_of(merged);
            // 正常路径下 after 以 before 为前缀（Text/Reasoning 为追加语义）；
            // 若后端改发全量快照，strip_prefix 失败即退化为整段输出，不会丢字。
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

        match mtype {
            MessageType::Text => {
                // 只回显模型正文：用户消息由 CLI 自己输出，工具结果文本不回显
                // （role 尚未到达的早期补丁按"助手"处理，避免漏字）。
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
                if self.tools_announced.insert(patch.id.clone()) {
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

        if failed {
            if let Some(err) = error.filter(|e| !e.is_empty()) {
                if self.failures_announced.insert(patch.id.clone()) {
                    if self.wrote_text {
                        write_stdout("\n");
                        self.wrote_text = false;
                    }
                    write_stderr(&format!("✖ 失败: {err}"));
                }
            }
        }
    }

    /// 后端精确删除某条消息（工具恢复时清理旧子节点）——本 CLI 不做消息树持久渲染，忽略。
    pub fn on_delete(&mut self, message_id: &str) {
        self.msgs.remove(message_id);
    }

    /// 会话运行态提示。
    ///
    /// 入参是**会话节点的状态词**（`VDFS_STATUS_*`），不再是旧事件频道的
    /// `busy` / `idle` —— 旧的 `other => "· 状态: …"` 兜底也一并删了：节点状态词
    /// 是闭集，能走到那里的只有未知词，而它不值得占一行——真正需要用户看到的失败
    /// 已经在 [`Self::on_update`]（消息级）与调用方的结局判定里报过。
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
    /// 消费端遇到「本该能处理但缺载荷」的变更时据此留痕：静默跳过是事故，
    /// 而把判断留在调用方就会在每个调用点各写一次 `quiet` 分支。
    pub fn warn(&self, msg: &str) {
        if !self.quiet {
            write_stderr(msg);
        }
    }
}
