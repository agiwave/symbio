//! Docker 执行器模块
//!
//! 提供基于 Docker 的代码执行能力

use std::process::Command;
use std::time::Instant;

use super::config::{ExecutionConfig, ExecutionResult};
use super::security::is_dangerous_command;

/// Docker 执行器
pub struct DockerExecutor {
    config: ExecutionConfig,
}

impl DockerExecutor {
    /// 创建新的执行器
    pub fn new(config: ExecutionConfig) -> Self {
        Self { config }
    }

    /// 使用默认配置创建执行器
    pub fn with_defaults() -> Self {
        Self::new(ExecutionConfig::default())
    }

    /// 执行命令
    pub fn execute(&self, command: &str) -> Result<ExecutionResult, String> {
        // 安全检查
        if is_dangerous_command(command) {
            return Err("Dangerous command detected".to_string());
        }

        // 检查 Docker 是否可用
        if !self.is_docker_available() {
            return Err("Docker is not available".to_string());
        }

        // 构建执行命令
        let mut args = self.config.to_docker_args();
        args.push(command.to_string());

        let start = Instant::now();

        // 执行命令
        let output = Command::new("docker")
            .args(&args)
            .output()
            .map_err(|e| format!("Failed to execute docker command: {}", e))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        // 构建结果
        let result = ExecutionResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration_ms,
            timed_out: false,
        };

        Ok(result)
    }

    /// 执行脚本文件
    pub fn execute_script(&self, script_path: &str, language: &str) -> Result<ExecutionResult, String> {
        let command = match language {
            "python" | "python3" => format!("python3 {}", script_path),
            "r" | "R" => format!("Rscript {}", script_path),
            "bash" | "shell" => format!("bash {}", script_path),
            _ => return Err(format!("Unsupported language: {}", language)),
        };

        self.execute(&command)
    }

    /// 检查 Docker 是否可用
    pub fn is_docker_available(&self) -> bool {
        Command::new("docker")
            .arg("info")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// 构建镜像
    #[allow(dead_code)]
    pub fn build_image(&self, dockerfile_path: &str, tag: &str) -> Result<(), String> {
        let output = Command::new("docker")
            .args(["build", "-t", tag, "-f", dockerfile_path, "."])
            .output()
            .map_err(|e| format!("Failed to build docker image: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }

    /// 检查镜像是否存在
    #[allow(dead_code)]
    pub fn image_exists(&self, tag: &str) -> bool {
        Command::new("docker")
            .args(["image", "inspect", tag])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// 获取执行配置
    #[allow(dead_code)]
    pub fn config(&self) -> &ExecutionConfig {
        &self.config
    }
}

impl Default for DockerExecutor {
    fn default() -> Self {
        Self::with_defaults()
    }
}
