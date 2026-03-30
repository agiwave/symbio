//! Docker 执行插件

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use super::execution::{DockerExecutor, ExecutionConfig, is_dangerous_command};
use serde_json::{Value, json};
use std::process::Command;
use std::time::Instant;

#[derive(Clone)]
pub struct DockerPlugin {
    meta: PluginMeta,
    config: ExecutionConfig,
}

impl DockerPlugin {
    pub fn new() -> Self {
        DockerPlugin {
            meta: PluginMeta {
                name: "docker".to_string(),
                description: "Docker 代码执行环境".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["available", "execute", "execute_script"],
                            "description": "执行动作类型"
                        },
                        "command": {
                            "type": "string",
                            "description": "要执行的命令 (action=execute)"
                        },
                        "script_path": {
                            "type": "string",
                            "description": "脚本文件路径 (action=execute_script)"
                        },
                        "language": {
                            "type": "string",
                            "enum": ["python", "python3", "r", "R", "bash", "sh", "shell"],
                            "description": "脚本语言 (action=execute_script)"
                        },
                        "config": {
                            "type": "object",
                            "properties": {
                                "cpu_limit": { "type": "number" },
                                "memory_limit": { "type": "number" },
                                "time_limit": { "type": "number" },
                                "network_disabled": { "type": "boolean" },
                                "image": { "type": "string" }
                            }
                        }
                    },
                    "required": ["action"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "exit_code": { "type": "number" },
                        "stdout": { "type": "string" },
                        "stderr": { "type": "string" },
                        "duration_ms": { "type": "number" },
                        "timed_out": { "type": "boolean" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            config: ExecutionConfig::default(),
        }
    }

    /// 检查 Docker 是否可用
    fn check_available(&self) -> Value {
        let available = Command::new("docker")
            .arg("info")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        
        json!({
            "success": true,
            "available": available
        })
    }

    /// 执行命令
    fn execute_command(&self, command: &str, config: Option<&Value>) -> PluginResult<InvokeStream> {
        // 安全检查
        if is_dangerous_command(command) {
            return Err(PluginError::ValidationError("危险命令被拦截".to_string()));
        }

        // 解析配置
        let exec_config = self.parse_config(config);
        let executor = DockerExecutor::new(exec_config);

        // 检查 Docker 是否可用
        if !executor.is_docker_available() {
            return Err(PluginError::InternalError("Docker 不可用".to_string()));
        }

        let start = Instant::now();

        // 执行命令
        match executor.execute(command) {
            Ok(result) => {
                Ok(InvokeStream::single(json!({
                    "success": result.exit_code == 0 && !result.timed_out,
                    "exit_code": result.exit_code,
                    "stdout": result.stdout,
                    "stderr": result.stderr,
                    "duration_ms": result.duration_ms,
                    "timed_out": result.timed_out
                })))
            }
            Err(e) => {
                Ok(InvokeStream::single(json!({
                    "success": false,
                    "exit_code": -1,
                    "stdout": "",
                    "stderr": e,
                    "duration_ms": start.elapsed().as_millis() as u64,
                    "timed_out": false
                })))
            }
        }
    }

    /// 执行脚本
    fn execute_script(&self, script_path: &str, language: &str, config: Option<&Value>) -> PluginResult<InvokeStream> {
        let exec_config = self.parse_config(config);
        let executor = DockerExecutor::new(exec_config);

        if !executor.is_docker_available() {
            return Err(PluginError::InternalError("Docker 不可用".to_string()));
        }

        let start = Instant::now();

        match executor.execute_script(script_path, language) {
            Ok(result) => {
                Ok(InvokeStream::single(json!({
                    "success": result.exit_code == 0 && !result.timed_out,
                    "exit_code": result.exit_code,
                    "stdout": result.stdout,
                    "stderr": result.stderr,
                    "duration_ms": result.duration_ms,
                    "timed_out": result.timed_out
                })))
            }
            Err(e) => {
                Ok(InvokeStream::single(json!({
                    "success": false,
                    "exit_code": -1,
                    "stdout": "",
                    "stderr": e,
                    "duration_ms": start.elapsed().as_millis() as u64,
                    "timed_out": false
                })))
            }
        }
    }

    /// 解析配置
    fn parse_config(&self, config: Option<&Value>) -> ExecutionConfig {
        match config {
            Some(cfg) => {
                let mut exec_config = ExecutionConfig::default();
                if let Some(cpu) = cfg.get("cpu_limit").and_then(|v| v.as_f64()) {
                    exec_config.cpu_limit = cpu as f32;
                }
                if let Some(mem) = cfg.get("memory_limit").and_then(|v| v.as_u64()) {
                    exec_config.memory_limit = mem;
                }
                if let Some(time) = cfg.get("time_limit").and_then(|v| v.as_u64()) {
                    exec_config.time_limit = time;
                }
                if let Some(network) = cfg.get("network_disabled").and_then(|v| v.as_bool()) {
                    exec_config.network_disabled = network;
                }
                if let Some(image) = cfg.get("image").and_then(|v| v.as_str()) {
                    exec_config.image = image.to_string();
                }
                exec_config
            }
            None => self.config.clone(),
        }
    }
}

#[async_trait::async_trait]
impl Plugin for DockerPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(self.meta.clone())
        } else {
            Err(PluginError::NotFound(format!("插件路径 '{}' 未找到", path)))
        }
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if !path.is_empty() {
            return Err(PluginError::NotFound(format!("插件路径 '{}' 未找到", path)));
        }

        let obj = input.as_object()
            .ok_or_else(|| PluginError::ValidationError("输入必须是对象".to_string()))?;

        let action = obj.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 action 字段".to_string()))?;

        match action {
            "available" => Ok(InvokeStream::single(self.check_available())),
            "execute" => {
                let command = obj.get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 command 字段".to_string()))?;
                self.execute_command(command, obj.get("config"))
            }
            "execute_script" => {
                let script_path = obj.get("script_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 script_path 字段".to_string()))?;
                let language = obj.get("language")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 language 字段".to_string()))?;
                self.execute_script(script_path, language, obj.get("config"))
            }
            _ => Err(PluginError::ValidationError(format!("未知 action: {}", action))),
        }
    }
}

impl Default for DockerPlugin {
    fn default() -> Self {
        Self::new()
    }
}
