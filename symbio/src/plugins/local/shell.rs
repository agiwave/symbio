//! Shell 命令执行工具 - 实现 Tool trait
//!
//! ## 安全模型
//!
//! - `command` 字段是用户/Agent 提供的**整条 shell 命令**，不是部分插值。
//!   因此不构成"把不可信数据拼进固定命令"的注入模式。
//! - 真正的注入防御靠 `SecurityPolicy::is_command_allowed`（白名单）+ 风险等级判定。
//! - 中/高风险命令必须 `approved: true` 才会执行（AutonomyLevel::Supervised 模式）。
//! - 执行通过 `tokio::process::Command` 显式 spawn shell（`cmd /C` 或 `sh -c`），
//!   没有把任何不可信片段拼到固定的 argv 里。
//! - Unix 上进一步清空 env，只透传白名单变量，减小环境变量泄漏面。
//!
//! ## 流式输出
//!
//! 只有一条执行路径（不再有「流式 / 非流式」两条）：
//! - `spawn` 子进程，stdout/stderr 各由一个 pump 任务按行读取；
//! - 每行到达后向**执行期出口** `sink` 发 `NodeOp::Upsert`
//!   （`role=tool` + `status=Streaming`，**累积全量快照**——对应协议里 `upsert`
//!   的「按 id 整条替换」语义，消费端不做任何合并）；
//! - 进程结束后把完整输出作为 `Data` 返回，由 `tool_executor` 定稿为工具结果；
//! - 中止（`AbortSignal`）/超时（`SHELL_TIMEOUT_SECS`）时 kill 子进程并收尸。
//!
//! ## 为什么这里曾经很复杂
//!
//! 出口曾是「返回值里的 `PluginPayload::Session` 通道」——于是执行体**必须**
//! `spawn` 到后台：executor 要先拿到 `Session(rx)` 才会开始消费，而 `execute()`
//! 不返回就没人消费；输出超过通道容量（64 帧）时 pump 阻塞在 `send`、`execute()`
//! 永不返回 ⇒ **死锁**。出口改成 `ctx` 注入的 `EventSink` 后，`emit` 不会阻塞在
//! 无界背压上，「先返回还是先执行」这个顺序问题连同通道容量、`cancel_token`
//! 克隆、「send 失败 ⇒ 消费端已消失」标志一起消失。
//!
//! 出口缺席（`route()` 直连调用，如 MCP 网关）⇒ `EventSink::of` 给 `Null`：
//! 同一份代码照跑，只是不发增量。
use super::policy::{RiskLevel, SecurityPolicy};
use super::system::{decode_output, validate_params};
use crate::symbio_core::{
    schemas::session::chat_message::{
        ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
    },
    schemas::session::session_chat_response,
    AbortSignal, Capability, CapabilityMeta, EventSink, ExecEnv, InvokeRequest, InvokeRequestExt,
    PluginError,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

const SHELL_TIMEOUT_SECS: u64 = 3600;
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// 流式快照帧的节流间隔：避免高频输出（如 ping/大文件 cat）打爆通道与前端
const STREAM_EMIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub exit_code: Option<i32>,
    pub output: String,
    pub risk_level: String,
}

/// 增量快照的**节点身份**（编排层提供）。
///
/// 两个 id 必须同行：`msg_id` 是编排层为本次调用预留的结果节点（占位节点也是它建的），
/// `tool_call_id` 是父 ToolCall。工具自己猜 id 正是历史上「同一逻辑节点两个 id」的成因，
/// 因此这里只接受编排层给的一对值。
///
/// **缺席 = 直连调用**（`route()`，如 MCP 网关）：没有占位节点，也就不发增量快照——
/// 与 [`EventSink::Null`] 是同一件事的两个面（都由「编排层是否在场」决定）。
#[derive(Debug, Clone)]
struct SnapshotTarget {
    msg_id: String,
    tool_call_id: String,
}

impl SnapshotTarget {
    /// 从 ctx 读；任一 id 缺失 ⇒ `None`。
    fn from_ctx(ctx: &dyn InvokeRequest) -> Option<Self> {
        let msg_id = ctx
            .get(crate::symbio_core::RESULT_MSG_ID)
            .unwrap_or_default();
        let tool_call_id = ctx
            .get(crate::symbio_core::TOOL_CALL_ID)
            .unwrap_or_default();
        if msg_id.is_empty() || tool_call_id.is_empty() {
            return None;
        }
        Some(Self {
            msg_id,
            tool_call_id,
        })
    }
}

/// 获取当前操作系统信息
fn get_os_info() -> (&'static str, &'static str, &'static str, &'static str) {
    // 返回: (os_name, shell_name, tool_name, example_commands)
    // tool_name 是实际执行命令的工具名称（用于工具注册）
    #[cfg(target_os = "windows")]
    {
        (
            "Windows",
            "cmd.exe",
            "cmd",
            "dir, type, copy, del, mkdir, rmdir, where, findstr",
        )
    }
    #[cfg(target_os = "macos")]
    {
        (
            "macOS",
            "zsh/sh",
            "sh",
            "ls, cat, cp, rm, mkdir, rmdir, which, grep",
        )
    }
    #[cfg(target_os = "linux")]
    {
        (
            "Linux",
            "sh/bash",
            "sh",
            "ls, cat, cp, rm, mkdir, rmdir, which, grep",
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        (
            "Unknown",
            "sh",
            "sh",
            "ls, cat, cp, rm, mkdir, rmdir, which, grep",
        )
    }
}

/// Shell 命令执行工具
#[derive(Clone)]
pub struct ShellTool {
    security: Arc<SecurityPolicy>,
}

impl ShellTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    /// 公共前置：参数校验 + 速率限制 + 风险等级判定 + 动作记录。
    /// 返回 `(command, risk)`。
    fn prepare(
        &self,
        args: &Value,
        threshold: RiskLevel,
    ) -> Result<(String, RiskLevel), PluginError> {
        validate_params(args, &["command"]).map_err(PluginError::ValidationError)?;

        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(cmd) if !cmd.is_empty() => cmd.to_string(),
            _ => {
                return Err(PluginError::ValidationError(
                    "Missing or empty 'command' argument".into(),
                ))
            }
        };

        let approved = args
            .get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // 检查速率限制
        if self.security.is_rate_limited() {
            return Err(PluginError::InternalError("速率限制：操作过于频繁".into()));
        }

        // 验证命令（per-session 风险等级阈值从 ctx[RISK_LEVEL] 透传而来）
        let risk = self
            .security
            .validate_command_execution(&command, approved, threshold)
            .map_err(PluginError::InternalError)?;

        // 记录动作
        self.security.record_action();

        Ok((command, risk))
    }

    /// 构建按操作系统分派的 shell 命令（不含 workdir/env 设置）
    fn build_command(command: &str) -> tokio::process::Command {
        #[cfg(target_os = "windows")]
        let cmd = {
            let mut c = tokio::process::Command::new("cmd");
            c.arg("/C").arg(command);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let cmd = {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(command);
            c
        };
        cmd
    }

    /// 执行命令：spawn 子进程、按行广播增量快照、返回完整输出。
    ///
    /// - `sink`：执行期出口（缺席时是 `Null`，同一份代码不发增量）
    /// - `abort`：中止信号；`abort()` 置位即 kill 子进程
    /// - `target`：增量快照的节点身份（编排层提供，可能为空——直连调用没有占位节点）
    ///
    /// 返回 [`Response`]（`exit_code` / `output` / `risk_level`），由
    /// `execute_tool_async` 经 `extract_result` 取 `output` 回传 LLM。
    #[allow(clippy::too_many_arguments)]
    async fn execute_streaming(
        &self,
        args: &Value,
        workdir: &str,
        threshold: RiskLevel,
        sink: &EventSink,
        abort: &AbortSignal,
        target: Option<&SnapshotTarget>,
    ) -> Result<Response, PluginError> {
        let (command, risk) = self.prepare(args, threshold)?;

        let workspace_dir = std::path::PathBuf::from(shellexpand::tilde(workdir).to_string());

        let mut cmd = Self::build_command(&command);
        cmd.current_dir(&*workspace_dir);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Windows 不清理环境变量（需要 PATH 等）
        #[cfg(not(target_os = "windows"))]
        {
            const SAFE_ENV_VARS: &[&str] =
                &["PATH", "HOME", "USER", "SHELL", "LANG", "LC_ALL", "TERM"];
            cmd.env_clear();
            for var in SAFE_ENV_VARS {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| PluginError::InternalError(format!("命令执行失败: {e}")))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // 共享累积缓冲（stdout 在前，stderr 以 [stderr] 段缀尾——与非流式输出格式一致）
        let stdout_acc: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let stderr_acc: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

        let mut pump_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        if let Some(out) = stdout {
            pump_handles.push(pump_lines(
                out,
                stdout_acc.clone(),
                stderr_acc.clone(),
                false,
                sink.clone(),
                abort.clone(),
                target.cloned(),
            ));
        }
        if let Some(err) = stderr {
            pump_handles.push(pump_lines(
                err,
                stderr_acc.clone(),
                stdout_acc.clone(),
                true,
                sink.clone(),
                abort.clone(),
                target.cloned(),
            ));
        }

        // 等待 EOF（正常结束）或超时/中止（kill 后管道关闭 → pump EOF）
        tokio::select! {
            // pump 任务全部退出（EOF）即认为输出读取完毕
            _ = async {
                for h in pump_handles.iter_mut() {
                    let _ = h.await;
                }
            } => {}
            _ = tokio::time::sleep(Duration::from_secs(SHELL_TIMEOUT_SECS)) => {
                let _ = child.kill().await;
            }
            _ = abort.cancelled() => {
                let _ = child.kill().await;
            }
        }
        // 收尾：确保 pump 全部退出 + 子进程被回收（kill 后管道关闭，pump 很快 EOF）。
        // 注意：select 分支可能已消费部分 JoinHandle（await 返回 Ready），
        // 对已完成的 JoinHandle 再次 await 会 panic（polled after completion），
        // 因此只等待尚未结束的 pump。
        for h in pump_handles.iter_mut() {
            if !h.is_finished() {
                let _ = h.await;
            }
        }
        let status = child.wait().await;

        // 完整输出即**返回值**——不再需要"哨兵帧"：执行期没有帧面可言
        //（出口只承载增量快照，终态由 `tool_executor` 定稿）。
        let stdout_text = stdout_acc.lock().unwrap().clone();
        let stderr_text = stderr_acc.lock().unwrap().clone();
        let mut full = compose_output(&stdout_text, &stderr_text);
        // 最终截断：落在字符边界上，避免中文输出被硬切导致 panic
        if full.len() > MAX_OUTPUT_BYTES {
            full.truncate(crate::symbio_core::floor_char_boundary(
                &full,
                MAX_OUTPUT_BYTES,
            ));
            full.push_str("\n... [输出已截断]");
        }
        let exit_note = match status.as_ref() {
            Ok(s) => match s.code() {
                Some(code) => format!("\n[exit code: {code}]"),
                None => "\n[exit code: terminated by signal]".to_string(),
            },
            Err(_) => "\n[exit code: unknown]".to_string(),
        };
        full.push_str(&exit_note);

        Ok(Response {
            exit_code: status.as_ref().ok().and_then(|s| s.code()),
            output: full,
            risk_level: risk_level_str(&risk),
        })
    }
}

/// 按行读取管道，累积到共享缓冲并节流广播累积快照。
///
/// `own`/`other`：本管道/对侧管道的累积缓冲（is_stderr=false 时 own=stdout）。
/// `sink`：执行期出口；`Null` 时同一份代码只是不发增量（`emit` 是 no-op），
/// 因此这里**没有**「消费端是否还在」的判断——没有可关闭的通道，也就没有
/// 「send 失败」这种信号。
#[allow(clippy::too_many_arguments)]
fn pump_lines<R>(
    reader: R,
    own: Arc<Mutex<String>>,
    other: Arc<Mutex<String>>,
    is_stderr: bool,
    sink: EventSink,
    abort: AbortSignal,
    target: Option<SnapshotTarget>,
) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut buf: Vec<u8> = Vec::new();
        let mut last_emit = tokio::time::Instant::now() - STREAM_EMIT_INTERVAL;
        loop {
            buf.clear();
            let n = tokio::select! {
                biased;
                _ = abort.cancelled() => break,
                r = reader.read_until(b'\n', &mut buf) => r,
            };
            match n {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = decode_output(&buf);
                    {
                        let mut a = own.lock().unwrap();
                        // 硬上限保护：只保留前 MAX_OUTPUT_BYTES，超出部分丢弃
                        // （避免 cat 大文件把内存打爆；最终帧无需再截断）
                        if a.len() < MAX_OUTPUT_BYTES {
                            let remain = MAX_OUTPUT_BYTES - a.len();
                            let take = crate::symbio_core::floor_char_boundary(&line, remain);
                            if take > 0 {
                                a.push_str(&line[..take]);
                            }
                        }
                    }
                    // 节流广播累积快照（全量内容，前端 role=tool 全量替换）
                    if last_emit.elapsed() >= STREAM_EMIT_INTERVAL {
                        last_emit = tokio::time::Instant::now();
                        let Some(target) = target.as_ref() else {
                            continue;
                        };
                        let snapshot = {
                            let (a, b) = if is_stderr {
                                (other.lock().unwrap(), own.lock().unwrap())
                            } else {
                                (own.lock().unwrap(), other.lock().unwrap())
                            };
                            compose_output(&a, &b)
                        };
                        sink.emit(session_chat_response::NodeOp::Upsert {
                            message: Box::new(ChatMessage {
                                id: target.msg_id.clone(),
                                parent_id: Some(target.tool_call_id.clone()),
                                role: Some(MessageRole::Tool),
                                msg_type: Some(MessageType::Text),
                                content: Some(MessageContent::Text(snapshot)),
                                status: Some(MessageStatus::Streaming),
                                ..Default::default()
                            }),
                        })
                        .await;
                    }
                }
            }
        }
    })
}

/// 组合 stdout/stderr 为统一输出文本（与非流式格式一致）
fn compose_output(stdout: &str, stderr: &str) -> String {
    if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{stdout}\n[stderr]\n{stderr}")
    }
}

fn risk_level_str(risk: &RiskLevel) -> String {
    match risk {
        RiskLevel::Low => "low".to_string(),
        RiskLevel::Medium => "medium".to_string(),
        RiskLevel::High => "high".to_string(),
    }
}

#[async_trait]
impl Capability for ShellTool {
    fn meta(&self) -> CapabilityMeta {
        let (_os_name, shell_name, tool_name, example_commands) = get_os_info();
        let description = format!("执行操作系统 {shell_name} 命令");
        CapabilityMeta {
            name: tool_name.to_string(),
            description,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "要执行的 Shell 命令。请根据当前操作系统使用正确的命令语法。"
                    },
                },
                "required": ["command"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::SystemOperation),
            examples: Some(vec![
                format!(
                    "command='{}'",
                    example_commands.split(", ").next().unwrap_or("dir")
                ),
                "command='git status'".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn InvokeRequest>,
    ) -> Result<Value, PluginError> {
        let workdir_str = ctx.get(crate::symbio_core::WORKDIR).ok_or_else(|| {
            PluginError::ValidationError("Missing workdir in context".to_string())
        })?;
        if workdir_str.is_empty() {
            return Err(PluginError::ValidationError(
                "Empty workdir in context".to_string(),
            ));
        }
        // per-session 风险等级阈值：与 agent_id/provider_id/mode 同级别，由 orchestrator 写入 ctx
        let threshold = ctx
            .get(crate::symbio_core::RISK_LEVEL)
            .map(|s| match s.as_str() {
                "low" => RiskLevel::Low,
                "high" => RiskLevel::High,
                _ => RiskLevel::Medium,
            })
            .unwrap_or(RiskLevel::Medium);

        // 执行期双原语 + 快照身份，全部由编排层经 ctx 注入；**缺席即降级**：
        // `EventSink::Null`（不发增量）、永不中止的信号、`None` 快照身份。
        // 于是「会话里跑」与「被 route() 直连调用」共用同一条代码路径——
        // 不再有第二条分支、不再需要后台 spawn（没有通道容量可阻塞）。
        let sink = EventSink::of(&*ctx);
        let abort = AbortSignal::of(&*ctx);
        let target = SnapshotTarget::from_ctx(&*ctx);

        let resp = self
            .execute_streaming(
                &args,
                &workdir_str,
                threshold,
                &sink,
                &abort,
                target.as_ref(),
            )
            .await?;
        Ok(serde_json::to_value(&resp)?)
    }
}
#[cfg(test)]
#[path = "shell.test.rs"]
mod tests;
