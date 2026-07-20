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

use super::policy::{RiskLevel, SecurityPolicy};
use crate::symbio_core::{
    decode_output, validate_params, Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt,
    InvokeResponse, PluginError, PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const SHELL_TIMEOUT_SECS: u64 = 60;
const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// 获取当前操作系统信息
fn get_os_info() -> (&'static str, &'static str, &'static str, &'static str) {
    // 返回: (os_name, shell_name, tool_name, example_commands)
    // tool_name 是实际执行命令的工具名称（用于工具注册）
    #[cfg(target_os = "windows")]
    {
        (
            "Windows",
            "cmd.exe",
            "cmd.exe",
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

    async fn execute_inner(
        &self,
        args: Value,
        workdir: &str,
        threshold: RiskLevel,
    ) -> Result<Value, PluginError> {
        validate_params(&args, &["command"]).map_err(PluginError::ValidationError)?;

        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(cmd) if !cmd.is_empty() => cmd,
            _ => {
                return Err(PluginError::ValidationError(
                    "Missing or empty 'command' argument".into(),
                ))
            },
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
            .validate_command_execution(command, approved, threshold)
            .map_err(PluginError::InternalError)?;

        // 记录动作
        self.security.record_action();

        // 获取工作区目录
        let workspace_dir = std::path::PathBuf::from(shellexpand::tilde(workdir).to_string());

        // 构建命令 - 根据操作系统选择不同的 shell
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd");
            c.arg("/C").arg(command);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(command);
            c
        };

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
                // use crate::symbio_core::system::decode_output; // Removed duplicate or simplified path

                let mut stdout = decode_output(&output.stdout);
                let stderr = decode_output(&output.stderr);

                // 截断输出
                if stdout.len() > MAX_OUTPUT_BYTES {
                    stdout.truncate(MAX_OUTPUT_BYTES);
                    stdout.push_str("\n... [输出已截断]");
                }

                let full_output = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{stdout}\n[stderr]\n{stderr}")
                };

                Ok(serde_json::to_value(
                    crate::symbio_core::schemas::web::shell_execute::Response {
                        exit_code: output.status.code(),
                        output: full_output,
                        risk_level: match risk {
                            RiskLevel::Low => "low".to_string(),
                            RiskLevel::Medium => "medium".to_string(),
                            RiskLevel::High => "high".to_string(),
                        },
                    },
                )
                .unwrap_or_default())
            },
            Ok(Err(e)) => Err(PluginError::InternalError(format!("命令执行失败: {e}"))),
            Err(_) => Err(PluginError::InternalError(format!(
                "命令超时 ({SHELL_TIMEOUT_SECS}秒)"
            ))),
        }
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
        let result = self.execute_inner(args, &workdir_str, threshold).await?;
        Ok(PluginPayload::new(&result))
    }
}
