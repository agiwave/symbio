//! 执行配置模块

use serde::{Deserialize, Serialize};

/// 执行配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// CPU 核心数限制
    pub cpu_limit: f32,
    /// 内存限制 (MB)
    pub memory_limit: u64,
    /// 时间限制 (秒)
    pub time_limit: u64,
    /// 是否禁用网络
    pub network_disabled: bool,
    /// 只读路径
    pub read_only_paths: Vec<String>,
    /// 可写路径
    pub writable_paths: Vec<String>,
    /// 工作目录
    pub workdir: String,
    /// 镜像名称
    pub image: String,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            cpu_limit: 2.0,
            memory_limit: 4096,
            time_limit: 3600,
            network_disabled: true,
            read_only_paths: vec![],
            writable_paths: vec!["/workspace".to_string()],
            workdir: "/workspace".to_string(),
            image: "symbio-executor:latest".to_string(),
        }
    }
}

impl ExecutionConfig {
    /// 创建新的执行配置
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置 CPU 限制
    pub fn with_cpu_limit(mut self, limit: f32) -> Self {
        self.cpu_limit = limit;
        self
    }

    /// 设置内存限制
    pub fn with_memory_limit(mut self, limit: u64) -> Self {
        self.memory_limit = limit;
        self
    }

    /// 设置时间限制
    pub fn with_time_limit(mut self, limit: u64) -> Self {
        self.time_limit = limit;
        self
    }

    /// 设置镜像
    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    /// 转换为 Docker 运行参数
    pub fn to_docker_args(&self) -> Vec<String> {
        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            format!("--cpus={}", self.cpu_limit),
            format!("--memory={}m", self.memory_limit),
        ];

        if self.network_disabled {
            args.push("--network=none".to_string());
        }

        for path in &self.read_only_paths {
            args.push("-v".to_string());
            args.push(format!("{}:{}:ro", path, path));
        }

        for path in &self.writable_paths {
            args.push("-v".to_string());
            args.push(format!("{}:{}:rw", path, path));
        }

        args.push("-w".to_string());
        args.push(self.workdir.clone());

        args.push(self.image.clone());

        args
    }
}

/// 执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// 退出码
    pub exit_code: i32,
    /// 标准输出
    pub stdout: String,
    /// 标准错误
    pub stderr: String,
    /// 执行时间 (毫秒)
    pub duration_ms: u64,
    /// 是否超时
    pub timed_out: bool,
}

impl ExecutionResult {
    /// 创建成功结果
    pub fn success(stdout: String, duration_ms: u64) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr: String::new(),
            duration_ms,
            timed_out: false,
        }
    }

    /// 创建错误结果
    pub fn error(exit_code: i32, stderr: String, duration_ms: u64) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr,
            duration_ms,
            timed_out: false,
        }
    }

    /// 创建超时结果
    pub fn timeout(duration_ms: u64) -> Self {
        Self {
            exit_code: -1,
            stdout: String::new(),
            stderr: "Execution timed out".to_string(),
            duration_ms,
            timed_out: true,
        }
    }

    /// 是否成功
    pub fn is_success(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }
}
