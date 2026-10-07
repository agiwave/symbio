use crate::symbio_core::{HookEvent, HookOutput};
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::registry::{HookConfigEntry, HookExecutionResult, HookType};

pub struct HookExecutor {
    default_timeout_ms: u64,
}

impl HookExecutor {
    pub fn new() -> Self {
        Self {
            default_timeout_ms: 60000,
        }
    }

    pub async fn execute(
        &self,
        configs: &[HookConfigEntry],
        event: &HookEvent,
        _session_id: &str,
        workdir: &str,
    ) -> HookOutput {
        let mut aggregated_output = HookOutput::default();

        for config in configs {
            let result = self.execute_single(config, event, workdir).await;

            if !result.success && result.error.is_some() {
                return HookOutput::deny(result.error.unwrap_or_default());
            }

            if result.output.is_blocking() {
                return result.output;
            }

            if let Some(ctx) = result.output.context {
                if aggregated_output.context.is_some() {
                    let existing = aggregated_output.context.take().unwrap();
                    aggregated_output.context = Some(format!("{existing}\n\n{ctx}"));
                } else {
                    aggregated_output.context = Some(ctx);
                }
            }
        }

        aggregated_output
    }

    async fn execute_single(
        &self,
        config: &HookConfigEntry,
        event: &HookEvent,
        workdir: &str,
    ) -> HookExecutionResult {
        let timeout_ms = config.timeout_ms.unwrap_or(self.default_timeout_ms);

        match config.hook_type {
            HookType::Command => {
                self.execute_command(config, event, workdir, timeout_ms)
                    .await
            }
            HookType::Http => self.execute_http(config, event, timeout_ms).await,
        }
    }

    async fn execute_command(
        &self,
        config: &HookConfigEntry,
        event: &HookEvent,
        workdir: &str,
        timeout_ms: u64,
    ) -> HookExecutionResult {
        let command = match &config.command {
            Some(cmd) => cmd,
            None => {
                return HookExecutionResult {
                    success: false,
                    output: HookOutput::default(),
                    error: Some("No command specified".into()),
                };
            }
        };

        let event_json = match serde_json::to_string(event) {
            Ok(s) => s,
            Err(e) => {
                return HookExecutionResult {
                    success: false,
                    output: HookOutput::default(),
                    error: Some(format!("Failed to serialize event: {e}")),
                };
            }
        };

        // 载荷落**独立临时目录**（不是 workdir——那是用户的项目目录，钩子跑一次
        // 就在里面留一个文件），文件名由 tempfile 唯一生成：早先的
        // `.hook_event_<pid>.json` 只带进程号，同进程内并发钩子 / 残留文件会撞名，
        // 撞名就是「A 钩子读到 B 钩子的事件」。
        let tmp_dir = match tempfile::Builder::new().prefix("symbio-hook-").tempdir() {
            Ok(d) => d,
            Err(e) => {
                return HookExecutionResult {
                    success: false,
                    output: HookOutput::default(),
                    error: Some(format!("Failed to create temp dir: {e}")),
                };
            }
        };
        let event_file = tmp_dir.path().join("event.json");
        if let Err(e) = tokio::fs::write(&event_file, &event_json).await {
            return HookExecutionResult {
                success: false,
                output: HookOutput::default(),
                error: Some(format!("Failed to write event file: {e}")),
            };
        }

        // 事件文件路径作为**独立参数**追加（`cmd <path>` / `sh -c 'cmd' <path>`），
        // 而不是拼进命令串。两条理由都是实测出来的：
        // - **Windows**：命令串里内嵌 `"` 会被 Rust 的 MSVC 参数转义写成 `\"`，
        //   而 `cmd.exe` 不认反斜杠转义——路径于是变成 `C:\Temp\...\"`，直接报
        //   「文件名、目录名或卷标语法不正确」。走独立参数则由 Rust 负责转义，
        //   两边都不必手工加引号。
        // - **POSIX**：`sh -c <string> <name> <arg…>` 里第一个是 `$0`、第二个才是
        //   `$1`。故补一个占位 `$0`，事件路径才落在 `$1`——与钩子作者「读 `$1`」
        //   的直觉一致（漏掉占位，路径会占掉 `$0`，`$1` 为空）。
        //
        // 载荷 JSON **自始至终不进命令行**，故命令串里没有可被 shell 解释的内容。
        // tempfile 生成的名字不含空格 / 引号，两种平台都不会因空格而拆参。
        let event_path = event_file.to_string_lossy().into_owned();

        let output = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .arg("/C")
                .arg(command)
                .arg(&event_path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .current_dir(workdir)
                .output()
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .arg("symbio-hook") // 占位 `$0`
                .arg(&event_path) // `$1`
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .current_dir(workdir)
                .output()
        };

        let result = timeout(Duration::from_millis(timeout_ms), output).await;

        // 清理交给 `tmp_dir` 的 Drop（RAII）：早先把 `&& rm -f <file>` 拼进命令串，
        // 于是「清理」变成了命令的一部分——命令失败就不清理，Windows 上更是
        // 根本没有 `rm`。命令跑完（成败皆然）目录随 drop 消失。
        drop(tmp_dir);

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                let hook_output = if exit_code == 0 {
                    if let Ok(output) = serde_json::from_str::<HookOutput>(stdout.trim()) {
                        output
                    } else if !stdout.trim().is_empty() {
                        HookOutput::allow().with_context(stdout.trim())
                    } else {
                        HookOutput::allow()
                    }
                } else if exit_code == 1 {
                    let msg = if !stdout.trim().is_empty() {
                        stdout.trim()
                    } else {
                        &stderr
                    };
                    HookOutput::allow().with_context(format!("Warning: {msg}"))
                } else {
                    let msg = if !stdout.trim().is_empty() {
                        stdout.trim()
                    } else {
                        &stderr
                    };
                    HookOutput::deny(msg)
                };

                HookExecutionResult {
                    success: exit_code == 0,
                    output: hook_output,
                    error: None,
                }
            }
            Ok(Err(e)) => HookExecutionResult {
                success: false,
                output: HookOutput::default(),
                error: Some(format!("Command error: {e}")),
            },
            Err(_) => HookExecutionResult {
                success: false,
                output: HookOutput::default(),
                error: Some("Command timed out".into()),
            },
        }
    }

    async fn execute_http(
        &self,
        config: &HookConfigEntry,
        event: &HookEvent,
        timeout_ms: u64,
    ) -> HookExecutionResult {
        let url = match &config.url {
            Some(url) => url,
            None => {
                return HookExecutionResult {
                    success: false,
                    output: HookOutput::default(),
                    error: Some("No URL specified".into()),
                };
            }
        };

        let client = reqwest::Client::new();
        let event_json = serde_json::to_string(event).unwrap_or_default();

        let request = client
            .post(url)
            .body(event_json)
            .timeout(Duration::from_millis(timeout_ms));

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let exit_code = if status.is_success() { 0 } else { 1 };

                let hook_output = if exit_code == 0 {
                    if let Ok(output) = serde_json::from_str::<HookOutput>(&body) {
                        output
                    } else if !body.trim().is_empty() {
                        HookOutput::allow().with_context(body.trim())
                    } else {
                        HookOutput::allow()
                    }
                } else {
                    HookOutput::deny(format!("HTTP error {status}: {body}"))
                };

                HookExecutionResult {
                    success: status.is_success(),
                    output: hook_output,
                    error: None,
                }
            }
            Err(e) => HookExecutionResult {
                success: false,
                output: HookOutput::default(),
                error: Some(format!("HTTP request failed: {e}")),
            },
        }
    }
}

impl Default for HookExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "executor.test.rs"]
mod tests;
