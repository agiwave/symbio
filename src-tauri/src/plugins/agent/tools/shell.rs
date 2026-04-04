//! Shell 命令执行工具 - 实现 Plugin trait

use super::policy::{CommandRiskLevel, SecurityPolicy};
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const SHELL_TIMEOUT_SECS: u64 = 60;
const MAX_OUTPUT_BYTES: usize = 1_048_576;
const SAFE_ENV_VARS: &[&str] = &["PATH", "HOME", "TERM", "LANG", "LC_ALL", "USER", "SHELL"];

/// Shell 命令执行工具
pub struct ShellTool {
    security: Arc<SecurityPolicy>,
}

impl ShellTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "shell".to_string(),
            description: "执行 Shell 命令。命令在沙箱环境中运行，有超时限制。".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "要执行的 Shell 命令"
                    },
                    "approved": {
                        "type": "boolean",
                        "description": "是否已批准执行高风险命令"
                    }
                },
                "required": ["command"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "exit_code": { "type": "integer" },
                    "output": { "type": "string" },
                    "risk_level": { "type": "string" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    async fn execute_inner(&self, args: Value) -> Result<Value, PluginError> {
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

        // 获取工作区目录
        let workspace_dir = self.security.get_workspace_dir().await;

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
            cmd.env_clear();
            for var in SAFE_ENV_VARS {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
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

#[async_trait]
impl Plugin for ShellTool {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(Self::create_meta())
        } else {
            Err(PluginError::NotFound(format!("路径不存在: {}", path)))
        }
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if !path.is_empty() {
            return Err(PluginError::NotFound(format!("路径不存在: {}", path)));
        }

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match self.execute_inner(input).await {
                    Ok(result) => StreamChunk {
                        data: result,
                        done: true,
                        error: None,
                    },
                    Err(e) => StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(e.to_string()),
                    },
                }
            })
        });

        Ok(InvokeStream::Single(result))
    }
}