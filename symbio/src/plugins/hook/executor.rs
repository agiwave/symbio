use crate::symbio_core::schemas::system::hook::{HookEvent, HookOutput};
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
            },
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
            // SYS-003: 避免将 `Some(workdir)` 模式绑定名设置为 `workdir`，
            // 防止遮蔽外层同名参数导致后续 `workdir` 不再指向原参数。
            Some(cmd) => cmd,
            None => {
                return HookExecutionResult {
                    success: false,
                    output: HookOutput::default(),
                    error: Some("No command specified".into()),
                };
            },
        };

        let event_json = serde_json::to_string(event).unwrap_or_default();
        let event_file = format!("{}/.hook_event_{}.json", workdir, std::process::id());

        let write_result = tokio::fs::write(&event_file, &event_json).await;
        if let Err(e) = write_result {
            return HookExecutionResult {
                success: false,
                output: HookOutput::default(),
                error: Some(format!("Failed to write event file: {e}")),
            };
        }

        let shell = if cfg!(target_os = "windows") {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        let full_command = format!(
            "{} {} && rm -f {}",
            command,
            event_file
                .replace("/", std::path::MAIN_SEPARATOR.to_string().as_str())
                .replace("\\", "\\\\"),
            event_file
                .replace("/", std::path::MAIN_SEPARATOR.to_string().as_str())
                .replace("\\", "\\\\")
        );

        let output = Command::new(shell.0)
            .arg(shell.1)
            .arg(&full_command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(workdir)
            .output();

        let result = timeout(Duration::from_millis(timeout_ms), output).await;

        let _ = tokio::fs::remove_file(&event_file).await;

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
            },
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
            },
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
            },
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
