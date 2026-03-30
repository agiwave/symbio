//! 安全策略实现
//!
//! 提供沙箱边界、命令白名单和速率限制

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Agent 自主级别
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    /// 只读：只能观察，不能操作
    ReadOnly,
    /// 监督：可以操作，但危险操作需要批准
    #[default]
    Supervised,
    /// 完全自主：在策略范围内自主执行
    Full,
}

/// Shell 命令风险等级
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRiskLevel {
    /// 安全的只读命令 (ls, cat, grep 等)
    Low,
    /// 需要批准的状态修改命令 (git commit, npm install)
    Medium,
    /// 危险或特权命令 (rm -rf, sudo, chmod)
    High,
}

/// 滑动窗口动作追踪器
#[derive(Debug)]
pub struct ActionTracker {
    actions: Mutex<Vec<Instant>>,
    window_secs: u64,
}

impl ActionTracker {
    pub fn new() -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
            window_secs: 3600,
        }
    }

    pub fn record(&self) -> usize {
        let mut actions = self.actions.lock().expect("ActionTracker mutex poisoned");
        self.cleanup_old_actions(&mut actions);
        actions.push(Instant::now());
        actions.len()
    }

    pub fn count(&self) -> usize {
        let mut actions = self.actions.lock().expect("ActionTracker mutex poisoned");
        self.cleanup_old_actions(&mut actions);
        actions.len()
    }

    pub fn is_at_limit(&self, max_actions: u32) -> bool {
        self.count() >= max_actions as usize
    }

    fn cleanup_old_actions(&self, actions: &mut Vec<Instant>) {
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(self.window_secs))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
    }
}

impl Default for ActionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ActionTracker {
    fn clone(&self) -> Self {
        let actions = self.actions.lock().expect("ActionTracker mutex poisoned");
        Self {
            actions: Mutex::new(actions.clone()),
            window_secs: self.window_secs,
        }
    }
}

/// 工具执行安全策略
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// 当前自主级别
    pub autonomy: AutonomyLevel,
    /// 工作区目录
    pub workspace_dir: PathBuf,
    /// 是否限制只能访问工作区
    pub workspace_only: bool,
    /// 允许执行的命令列表
    pub allowed_commands: Vec<String>,
    /// 禁止访问的路径
    pub forbidden_paths: Vec<String>,
    /// 额外允许的根目录
    pub allowed_roots: Vec<PathBuf>,
    /// 每小时最大动作数
    pub max_actions_per_hour: u32,
    /// 中等风险命令是否需要批准
    pub require_approval_for_medium_risk: bool,
    /// 是否阻止高风险命令
    pub block_high_risk_commands: bool,
    /// 动作追踪器
    pub tracker: ActionTracker,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: PathBuf::from("."),
            workspace_only: false,
            allowed_commands: vec![
                "git".into(), "npm".into(), "cargo".into(),
                "ls".into(), "cat".into(), "grep".into(), "find".into(),
                "echo".into(), "pwd".into(), "wc".into(), "head".into(), "tail".into(),
                "date".into(), "python3".into(), "python".into(), "R".into(), "Rscript".into(),
            ],
            forbidden_paths: vec![
                "/etc".into(), "/root".into(), "/usr".into(),
                "~/.ssh".into(), "~/.gnupg".into(), "~/.aws".into(),
            ],
            allowed_roots: Vec::new(),
            max_actions_per_hour: 100,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            tracker: ActionTracker::new(),
        }
    }
}

impl SecurityPolicy {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            ..Self::default()
        }
    }

    pub fn read_only(workspace_dir: PathBuf) -> Self {
        Self {
            autonomy: AutonomyLevel::ReadOnly,
            workspace_dir,
            ..Self::default()
        }
    }

    pub fn full_autonomy(workspace_dir: PathBuf) -> Self {
        Self {
            autonomy: AutonomyLevel::Full,
            workspace_dir,
            ..Self::default()
        }
    }

    /// 检查路径是否被允许
    pub fn is_path_allowed<P: AsRef<Path>>(&self, path: P) -> bool {
        let path = path.as_ref();
        let path_str = path.to_string_lossy();

        // 检查路径遍历
        if path_str.contains("..") {
            return false;
        }

        // 检查禁止路径
        for forbidden in &self.forbidden_paths {
            let expanded = shellexpand::tilde(forbidden);
            if path_str.starts_with(expanded.as_ref()) {
                return false;
            }
        }

        if !self.workspace_only {
            return true;
        }

        path.starts_with(&self.workspace_dir) ||
            self.allowed_roots.iter().any(|r| path.starts_with(r))
    }

    /// 检查路径是否允许读取
    pub fn is_path_allowed_for_read<P: AsRef<Path>>(&self, path: P) -> bool {
        self.is_path_allowed(path)
    }

    /// 检查路径是否允许写入
    pub fn is_path_allowed_for_write<P: AsRef<Path>>(&self, path: P) -> bool {
        let path = path.as_ref();

        // 检查禁止路径
        if !self.is_path_allowed(path) {
            return false;
        }

        // 写入只允许在工作区
        path.starts_with(&self.workspace_dir) ||
            self.allowed_roots.iter().any(|r| path.starts_with(r))
    }

    /// 检查命令是否被允许
    pub fn is_command_allowed(&self, command: &str) -> bool {
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }

        // 阻止子shell操作符
        if command.contains('`') || command.contains("$(") {
            return false;
        }

        // 获取基础命令
        let base_cmd = command.split_whitespace().next().unwrap_or("");
        let cmd_name = base_cmd.rsplit('/').next().unwrap_or(base_cmd);

        self.allowed_commands.iter().any(|allowed| {
            allowed == cmd_name || allowed == base_cmd
        })
    }

    /// 检查是否可以执行动作
    pub fn can_act(&self) -> bool {
        self.autonomy != AutonomyLevel::ReadOnly && !self.is_rate_limited()
    }

    /// 检查是否达到速率限制
    pub fn is_rate_limited(&self) -> bool {
        self.tracker.is_at_limit(self.max_actions_per_hour)
    }

    /// 记录动作
    pub fn record_action(&self) {
        self.tracker.record();
    }

    /// 评估命令风险等级
    pub fn command_risk_level(&self, command: &str) -> CommandRiskLevel {
        let command_lower = command.to_lowercase();
        let base_cmd = command_lower.split_whitespace().next().unwrap_or("");

        // 高风险命令
        let high_risk = [
            "rm", "sudo", "su", "chmod", "chown", "shutdown", "reboot",
            "mkfs", "dd", "mount", "umount", "curl", "wget",
        ];
        if high_risk.contains(&base_cmd) {
            return CommandRiskLevel::High;
        }

        // 检查高风险模式
        let high_risk_patterns = ["rm -rf /", "rm -fr /", "mkfs", "dd if=", "sudo"];
        for pattern in &high_risk_patterns {
            if command_lower.contains(pattern) {
                return CommandRiskLevel::High;
            }
        }

        // 中等风险命令
        let medium_risk = ["touch", "mkdir", "mv", "cp", "ln"];
        if medium_risk.contains(&base_cmd) {
            return CommandRiskLevel::Medium;
        }

        // Git 操作
        if base_cmd == "git" {
            let git_ops = ["commit", "push", "reset", "clean", "rebase", "merge"];
            let second = command_lower.split_whitespace().nth(1).unwrap_or("");
            if git_ops.contains(&second) {
                return CommandRiskLevel::Medium;
            }
        }

        CommandRiskLevel::Low
    }

    /// 验证命令是否可执行
    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
    ) -> Result<CommandRiskLevel, String> {
        if !self.is_command_allowed(command) {
            return Err(format!("命令不在允许列表中: {}", command));
        }

        let risk = self.command_risk_level(command);

        match risk {
            CommandRiskLevel::High => {
                if self.block_high_risk_commands {
                    return Err("高风险命令被策略阻止".into());
                }
                if self.autonomy == AutonomyLevel::Supervised && !approved {
                    return Err("高风险命令需要显式批准".into());
                }
            }
            CommandRiskLevel::Medium => {
                if self.autonomy == AutonomyLevel::Supervised
                    && self.require_approval_for_medium_risk
                    && !approved
                {
                    return Err("中等风险命令需要批准".into());
                }
            }
            CommandRiskLevel::Low => {}
        }

        Ok(risk)
    }
}
