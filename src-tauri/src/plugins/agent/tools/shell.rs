//! Shell 命令执行工具

use super::policy::{CommandRiskLevel, SecurityPolicy};
use crate::core::types::PluginError;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const SHELL_TIMEOUT_SECS: u64 = 60;
const MAX_OUTPUT_BYTES: usize = 1_048_576;
const SAFE_ENV_VARS: &[&str] = &["PATH", "HOME", "TERM", "LANG", "LC_ALL", "USER", "SHELL"];

pub struct ShellTool {
    security: Arc<SecurityPolicy>,
}

impl ShellTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    pub async fn execute(&self, args: Value) -> Result<Value, PluginError> {
        let command = args.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 command 参数".to_string()))?;

        let approved = args.get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // 检查速率限制
        if self.security.is_rate_limited() {
            return Err(PluginError::InternalError("速率限制：操作过于频繁".into()));
        }

        // 验证命令
        let risk = self.security.validate_command_execution(command, approved)
            .map_err(|e| PluginError::InternalError(e))?;

        // 记录动作
        self.security.record_action();

        // 构建命令
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.current_dir(&self.security.workspace_dir);

        // 清理环境变量
        cmd.env_clear();
        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        // 执行命令（带超时）
        let result = tokio::time::timeout(
            Duration::from_secs(SHELL_TIMEOUT_SECS),
            cmd.output()
        ).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                // 截断输出
                if stdout.len() > MAX_OUTPUT_BYTES {
                    stdout.truncate(MAX_OUTPUT_BYTES);
                    stdout.push_str("\n... [输出已截断]");
                }

                let full_output = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{}\n[stderr]\n{}", stdout, stderr)
                };

                Ok(json!({
                    "success": output.status.success(),
                    "exit_code": output.status.code(),
                    "output": full_output,
                    "risk_level": match risk {
                        CommandRiskLevel::Low => "low",
                        CommandRiskLevel::Medium => "medium",
                        CommandRiskLevel::High => "high",
                    }
                }))
            }
            Ok(Err(e)) => Err(PluginError::InternalError(format!("命令执行失败: {}", e))),
            Err(_) => Err(PluginError::InternalError(
                format!("命令超时 ({}秒)", SHELL_TIMEOUT_SECS)
            )),
        }
    }
}
