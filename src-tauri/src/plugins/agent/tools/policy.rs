//! 安全策略实现
//!
//! 提供沙箱边界、命令白名单和速率限制

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Arc};
use std::time::Instant;
use tokio::sync::RwLock;

/// 规范化路径用于比较
/// 在 Windows 上，canonicalize 返回带有 `\\?\` 前缀的路径，需要统一处理
pub fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    // 移除 Windows UNC 路径前缀（如 \\?\）
    if path_str.starts_with("\\\\?\\") {
        PathBuf::from(&path_str[4..])
    } else {
        path.to_path_buf()
    }
}

/// 检查路径是否以另一个路径为前缀（规范化后比较）
fn path_starts_with_normalized(base: &Path, prefix: &Path) -> bool {
    let normalized_base = normalize_path_for_comparison(base);
    let normalized_prefix = normalize_path_for_comparison(prefix);
    normalized_base.starts_with(&normalized_prefix)
}

/// 检查相对路径是否是安全的（不包含危险的 .. 遍历）
pub fn is_safe_relative_path(path: &str) -> bool {
    // 拒绝直接的 .. 
    if path == ".." {
        return false;
    }
    // 允许 ./xxx 和 xxx 这样的路径
    // 拒绝 ../xxx
    if path.starts_with("../") || path.starts_with("..\\") {
        return false;
    }
    // 允许 xxx/../yyy 这样的路径，只要最终解析后是安全的
    // 这个检查会在 execute_inner 中进行
    true
}

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
    /// 工作区目录（共享引用，支持动态更新）
    pub workspace_dir: Arc<RwLock<PathBuf>>,
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
            workspace_dir: Arc::new(RwLock::new(PathBuf::from("."))),
            workspace_only: false,
            allowed_commands: vec![
                // 通用命令
                "git".into(), "npm".into(), "cargo".into(),
                "python3".into(), "python".into(), "R".into(), "Rscript".into(),
                "echo".into(), "date".into(),
                // Unix/Linux 命令
                "ls".into(), "cat".into(), "grep".into(), "find".into(),
                "pwd".into(), "wc".into(), "head".into(), "tail".into(),
                // Windows 命令
                "dir".into(), "type".into(), "findstr".into(), "where".into(),
                "cd".into(), "copy".into(), "xcopy".into(), "move".into(),
                "del".into(), "mkdir".into(), "rmdir".into(), "cls".into(),
                "ver".into(), "systeminfo".into(), "tasklist".into(),
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
            workspace_dir: Arc::new(RwLock::new(workspace_dir)),
            ..Self::default()
        }
    }

    /// 更新工作区目录
    pub async fn update_workspace_dir(&self, new_dir: PathBuf) {
        let mut dir = self.workspace_dir.write().await;
        *dir = new_dir;
    }

    /// 获取工作区目录的只读引用
    pub async fn get_workspace_dir(&self) -> tokio::sync::RwLockReadGuard<'_, PathBuf> {
        self.workspace_dir.read().await
    }

    /// 检查路径是否允许读取
    /// 
    /// 内部处理所有逻辑：相对路径安全性、禁止路径、工作区范围、canonicalize 后的绝对路径
    pub async fn is_path_allowed_for_read<P: AsRef<Path>>(&self, path: P) -> bool {
        let path = path.as_ref();
        let path_str = path.to_string_lossy();

        // 检查相对路径安全性（拒绝 ../ 遍历）
        if !is_safe_relative_path(&path_str) {
            return false;
        }

        // 检查禁止路径
        for forbidden in &self.forbidden_paths {
            let expanded = shellexpand::tilde(forbidden);
            if path_str.starts_with(expanded.as_ref()) {
                return false;
            }
        }

        // 如果不限制工作区，允许所有路径
        if !self.workspace_only {
            return true;
        }

        // 相对路径允许（完整验证在工具执行时进行）
        if !path.is_absolute() {
            return true;
        }

        // 绝对路径：检查是否在工作区内
        let workspace = self.workspace_dir.read().await;
        path_starts_with_normalized(path, &*workspace) ||
            self.allowed_roots.iter().any(|r| path_starts_with_normalized(path, r))
    }

    /// 检查路径是否允许写入
    /// 
    /// 内部处理所有逻辑：相对路径安全性、禁止路径、工作区范围、canonicalize 后的绝对路径
    pub async fn is_path_allowed_for_write<P: AsRef<Path>>(&self, path: P) -> bool {
        let path = path.as_ref();
        let path_str = path.to_string_lossy();

        // 检查相对路径安全性（拒绝 ../ 遍历）
        if !is_safe_relative_path(&path_str) {
            return false;
        }

        // 检查禁止路径
        for forbidden in &self.forbidden_paths {
            let expanded = shellexpand::tilde(forbidden);
            if path_str.starts_with(expanded.as_ref()) {
                return false;
            }
        }

        // 如果不限制工作区，允许所有路径
        if !self.workspace_only {
            return true;
        }

        // 相对路径允许（完整验证在工具执行时进行）
        if !path.is_absolute() {
            return true;
        }

        // 绝对路径：检查是否在工作区内
        let workspace = self.workspace_dir.read().await;
        path_starts_with_normalized(path, &*workspace) ||
            self.allowed_roots.iter().any(|r| path_starts_with_normalized(path, r))
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
