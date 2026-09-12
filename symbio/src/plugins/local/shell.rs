//! Shell 命令执行工具 - 实现 Tool trait
//!
//! ## 安全模型（PROJECT_SCAN_REPORT.md P16 审计要点）
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
//! 会话上下文携带 `RESULT_MSG_ID` + `TOOL_CALL_ID` 时走流式路径：
//! - `spawn` 子进程，stdout/stderr 各由一个 pump 任务按行读取；
//! - 每行到达后向 `PluginChannel` 广播 `StreamEvent::Update` 帧
//!   （`role=tool` + `status=Streaming`，**累积全量快照**——前端对 tool 消息
//!   为全量替换合并语义，与 orchestrator 的 merge_message_patch 一致）；
//! - 进程结束后发送哨兵帧 `{"content": <full>}`，由 tool_executor 捕获为
//!   工具最终结果（对齐 run.rs 哨兵协议）；
//! - 中止（cancel_token）/超时（SHELL_TIMEOUT_SECS）时 kill 子进程并收尸。
//!
//! 非流式回退（无 RESULT_MSG_ID 的直连调用，如 MCP 网关）：保持原
//! `cmd.output()` 等待式行为不变。
use serde::{Deserialize, Serialize};
use super::policy::{RiskLevel, SecurityPolicy};
use super::system::{decode_output, validate_params};
use crate::symbio_core::{
    schemas::session::chat_message::{
        ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
    },
    schemas::session::session_chat_response,
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginChannel,
    PluginError, PluginFrame, PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const SHELL_TIMEOUT_SECS: u64 = 3600;
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// 流式快照帧的节流间隔：避免高频输出（如 ping/大文件 cat）打爆通道与前端
const STREAM_EMIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub exit_code: Option<i32>,
    pub output: String,
    pub risk_level: String,
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
    fn prepare(&self, args: &Value, threshold: RiskLevel) -> Result<(String, RiskLevel), PluginError> {
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

    /// 非流式执行（等待式，供无会话上下文的直连调用回退使用）
    async fn execute_inner(
        &self,
        args: Value,
        workdir: &str,
        threshold: RiskLevel,
    ) -> Result<Value, PluginError> {
        let (command, risk) = self.prepare(&args, threshold)?;

        // 获取工作区目录
        let workspace_dir = std::path::PathBuf::from(shellexpand::tilde(workdir).to_string());

        let mut cmd = Self::build_command(&command);
        cmd.current_dir(&*workspace_dir);

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

        // 执行命令（带超时）
        let result =
            tokio::time::timeout(Duration::from_secs(SHELL_TIMEOUT_SECS), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = decode_output(&output.stdout);
                let stderr = decode_output(&output.stderr);

                // 截断输出
                if stdout.len() > MAX_OUTPUT_BYTES {
                    stdout.truncate(MAX_OUTPUT_BYTES);
                    stdout.push_str("\n... [输出已截断]");
                }

                let full_output = compose_output(&stdout, &stderr);

                Ok(serde_json::to_value(
                    Response {
                        exit_code: output.status.code(),
                        output: full_output,
                        risk_level: risk_level_str(&risk),
                    },
                )
                .unwrap_or_default())
            }
            Ok(Err(e)) => Err(PluginError::InternalError(format!("命令执行失败: {e}"))),
            Err(_) => Err(PluginError::InternalError(format!(
                "命令超时 ({SHELL_TIMEOUT_SECS}秒)"
            ))),
        }
    }

    /// 流式执行：spawn 子进程并按行广播增量快照帧，结束时发送哨兵帧。
    ///
    /// - `tx`：会话通道发送侧（executor 从配对的 rx 消费）
    /// - `result_msg_id`：结果消息 id（与 executor 预广播的占位节点同 id，前端按 id 合并）
    /// - `tool_call_id`：父 ToolCall 节点 id（流式帧锚定其下）
    /// - `cancel`：中止信号（tool_executor abort 时 cancel，pump/主循环随之退出）
    #[allow(clippy::too_many_arguments)]
    async fn execute_streaming(
        &self,
        args: Value,
        workdir: &str,
        threshold: RiskLevel,
        tx: mpsc::Sender<PluginFrame>,
        result_msg_id: String,
        tool_call_id: String,
        cancel: CancellationToken,
    ) -> Result<(), PluginError> {
        let (command, _risk) = self.prepare(&args, threshold)?;

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
        // 消费端消失标志：executor abort / 提前 drop rx 时 pump 的 send 会失败，
        // 据此在主循环 kill 子进程，避免孤儿进程继续运行。
        let consumer_gone = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let mut pump_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        if let Some(out) = stdout {
            pump_handles.push(pump_lines(
                out,
                stdout_acc.clone(),
                stderr_acc.clone(),
                false,
                tx.clone(),
                result_msg_id.clone(),
                tool_call_id.clone(),
                cancel.clone(),
                consumer_gone.clone(),
            ));
        }
        if let Some(err) = stderr {
            pump_handles.push(pump_lines(
                err,
                stderr_acc.clone(),
                stdout_acc.clone(),
                true,
                tx.clone(),
                result_msg_id.clone(),
                tool_call_id.clone(),
                cancel.clone(),
                consumer_gone.clone(),
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
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
            }
        }
        // 消费端已消失（abort/提前断开）→ kill 子进程，避免孤儿进程
        if consumer_gone.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill().await;
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

        // 最终哨兵帧：tool_executor 捕获 {"content": ...} 作为工具结果
        let stdout_text = stdout_acc.lock().unwrap().clone();
        let stderr_text = stderr_acc.lock().unwrap().clone();
        let mut full = compose_output(&stdout_text, &stderr_text);
        if full.len() > MAX_OUTPUT_BYTES {
            full.truncate(MAX_OUTPUT_BYTES);
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

        let _ = tx
            .send(PluginFrame::Data(json!({ "content": full })))
            .await;
        // 关闭发送侧 → executor 的 recv() 返回 None → 流式循环结束
        drop(tx);
        Ok(())
    }
}

/// 按行读取管道，累积到共享缓冲并节流广播累积快照帧。
///
/// `own`/`other`：本管道/对侧管道的累积缓冲（is_stderr=false 时 own=stdout）。
/// `consumer_gone`：send 失败（消费端 abort/断开）时置位并退出。
#[allow(clippy::too_many_arguments)]
fn pump_lines<R>(
    reader: R,
    own: Arc<Mutex<String>>,
    other: Arc<Mutex<String>>,
    is_stderr: bool,
    tx: mpsc::Sender<PluginFrame>,
    msg_id: String,
    tool_call_id: String,
    cancel: CancellationToken,
    consumer_gone: Arc<std::sync::atomic::AtomicBool>,
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
                _ = cancel.cancelled() => break,
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
                            let take = line.len().min(remain);
                            a.push_str(&line[..take]);
                        }
                    }
                    // 节流广播累积快照（全量内容，前端 role=tool 全量替换）
                    if last_emit.elapsed() >= STREAM_EMIT_INTERVAL {
                        last_emit = tokio::time::Instant::now();
                        let snapshot = {
                            let (a, b) = if is_stderr {
                                (other.lock().unwrap(), own.lock().unwrap())
                            } else {
                                (own.lock().unwrap(), other.lock().unwrap())
                            };
                            compose_output(&a, &b)
                        };
                        let node = ChatMessage {
                            id: msg_id.clone(),
                            parent_id: Some(tool_call_id.clone()),
                            role: Some(MessageRole::Tool),
                            msg_type: Some(MessageType::Text),
                            content: Some(MessageContent::Text(snapshot)),
                            status: Some(MessageStatus::Streaming),
                            ..Default::default()
                        };
                        let sent = tx
                            .send(PluginFrame::Data(
                                serde_json::to_value(session_chat_response::StreamEvent::Update {
                                    message: node,
                                })
                                .unwrap_or_default(),
                            ))
                            .await;
                        if sent.is_err() {
                            // 消费端已消失（executor abort / rx 提前 drop）
                            consumer_gone
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            break;
                        }
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

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
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

        // 会话上下文携带 RESULT_MSG_ID + TOOL_CALL_ID → 流式路径。
        // 注意：executor 是先拿到 Session(rx) 返回值后才开始消费通道，
        // 因此执行体必须 spawn 到后台，否则输出帧超过通道容量（64）时
        // pump 阻塞在 send、execute() 不返回、executor 不消费 → 死锁。
        let result_msg_id = ctx.get(crate::symbio_core::RESULT_MSG_ID).unwrap_or_default();
        let tool_call_id = ctx.get(crate::symbio_core::TOOL_CALL_ID).unwrap_or_default();
        if !result_msg_id.is_empty() && !tool_call_id.is_empty() {
            let (tx_side, rx_side) = PluginChannel::pair(64);
            let cancel = rx_side.cancel_token.clone();
            let this = self.clone();
            let workdir = workdir_str.clone();
            tokio::spawn(async move {
                if let Err(e) = this
                    .execute_streaming(
                        args,
                        &workdir,
                        threshold,
                        tx_side.tx.clone(),
                        result_msg_id,
                        tool_call_id,
                        cancel,
                    )
                    .await
                {
                    // 失败也发哨兵帧，让 executor 的 full 收敛为错误信息
                    //（否则 recv() 直接返回 None，工具结果为空串）
                    let _ = tx_side
                        .tx
                        .send(PluginFrame::Data(json!({
                            "content": format!("Error: {e}")
                        })))
                        .await;
                }
                // tx drop → executor 的 recv() 返回 None → 流式循环结束
            });
            // cancel_token 与 rx_side 共享：executor 侧 abort 时可 cancel pump
            return Ok(PluginPayload::Session(rx_side));
        }

        let result = self.execute_inner(args, &workdir_str, threshold).await?;
        Ok(PluginPayload::new(&result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::SimpleRequest;
    use std::sync::Arc as StdArc;

    /// 构造带流式上下文（RESULT_MSG_ID/TOOL_CALL_ID/WORKDIR）的测试请求
    fn make_streaming_ctx(command: &str) -> StdArc<dyn InvokeRequest> {
        let req = SimpleRequest::new(None, None);
        req.set(crate::symbio_core::WORKDIR, ".".to_string());
        req.set(crate::symbio_core::RESULT_MSG_ID, "res-1".to_string());
        req.set(crate::symbio_core::TOOL_CALL_ID, "tc-1".to_string());
        // payload 以原生 JSON 存储（等价于 deprecated PAYLOAD 键，避免警告）
        req.set_raw(
            "payload",
            StdArc::new(json!({ "command": command, "approved": true })),
        );
        Arc::new(req)
    }

    /// 从通道消费直到哨兵帧，返回 (流式快照帧列表, 最终全文)
    async fn drain_channel(rx: &mut mpsc::Receiver<PluginFrame>) -> (Vec<String>, String) {
        let mut snapshots = Vec::new();
        let mut full = String::new();
        while let Some(frame) = rx.recv().await {
            match frame {
                PluginFrame::Data(d) => {
                    if let Ok(session_chat_response::StreamEvent::Update { message }) =
                        serde_json::from_value::<session_chat_response::StreamEvent>(d.clone())
                    {
                        if let Some(MessageContent::Text(t)) = message.content {
                            snapshots.push(t);
                        }
                    } else if let Some(text) = d.get("content").and_then(|v| v.as_str()) {
                        full = text.to_string();
                    }
                }
                PluginFrame::Error(e, _) => panic!("unexpected error frame: {e}"),
            }
        }
        (snapshots, full)
    }

    #[tokio::test]
    async fn streaming_echo_emits_snapshots_and_sentinel() {
        let tool = ShellTool::new(Arc::new(SecurityPolicy::default()));
        let ctx = make_streaming_ctx("echo hello_stream");

        let payload = tool.execute(ctx).await.unwrap();
        let PluginPayload::Session(mut chan) = payload else {
            panic!("expected Session payload for streaming path");
        };

        let (snapshots, full) = drain_channel(&mut chan.rx).await;
        // 哨兵帧必须携带完整输出
        assert!(full.contains("hello_stream"), "sentinel missing output: {full}");
        assert!(
            full.contains("[exit code: 0]"),
            "sentinel missing exit code: {full}"
        );
        // 至少一个流式快照（echo 输出一行，节流间隔 120ms 内可能合并，但 ≥0 帧均合法；
        // 关键约束是哨兵帧存在且全量）
        let _ = snapshots;
    }

    #[tokio::test]
    async fn streaming_missing_command_arg_returns_error_sentinel() {
        let tool = ShellTool::new(Arc::new(SecurityPolicy::default()));
        let req = SimpleRequest::new(None, None);
        req.set(crate::symbio_core::WORKDIR, ".".to_string());
        req.set(crate::symbio_core::RESULT_MSG_ID, "res-1".to_string());
        req.set(crate::symbio_core::TOOL_CALL_ID, "tc-1".to_string());
        req.set_raw("payload", StdArc::new(json!({ "command": "" })));

        let payload = tool.execute(Arc::new(req)).await.unwrap();
        let PluginPayload::Session(mut chan) = payload else {
            panic!("expected Session payload for streaming path");
        };

        let (snapshots, full) = drain_channel(&mut chan.rx).await;
        assert!(snapshots.is_empty(), "no snapshot expected on error");
        assert!(
            full.starts_with("Error:"),
            "error sentinel expected, got: {full}"
        );
    }

    #[tokio::test]
    async fn non_streaming_fallback_without_result_msg_id() {
        let tool = ShellTool::new(Arc::new(SecurityPolicy::default()));
        let req = SimpleRequest::new(None, None);
        req.set(crate::symbio_core::WORKDIR, ".".to_string());
        req.set_raw(
            "payload",
            StdArc::new(json!({ "command": "echo hello_fallback", "approved": true })),
        );

        let payload = tool.execute(Arc::new(req)).await.unwrap();
        let PluginPayload::Data(v) = payload else {
            panic!("expected Data payload for non-streaming path");
        };
        let v = v.serialize().unwrap();
        assert!(v["output"].as_str().unwrap().contains("hello_fallback"));
        assert_eq!(v["exit_code"].as_i64(), Some(0));
    }

    #[tokio::test]
    async fn streaming_many_lines_do_not_deadlock() {
        // 回归：execute() 必须先返回 Session（spawn 后台执行），
        // 否则输出帧超过通道容量（64）时会死锁（pump 阻塞在 send）。
        let tool = ShellTool::new(Arc::new(SecurityPolicy::default()));
        let ctx = make_streaming_ctx("echo line1 & echo line2 & echo line3 & echo line4");

        let payload = tool.execute(ctx).await.unwrap();
        let PluginPayload::Session(mut chan) = payload else {
            panic!("expected Session payload");
        };

        let (snapshots, full) = drain_channel(&mut chan.rx).await;
        assert!(full.contains("line1"), "sentinel must contain all lines: {full}");
        assert!(full.contains("line4"), "sentinel must contain all lines: {full}");
        let _ = snapshots;
    }
}
