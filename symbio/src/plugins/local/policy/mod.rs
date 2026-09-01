use std::path::{Path, PathBuf};

mod policy_tracker;
mod policy_types;

pub use policy_tracker::ActionTracker;
pub use policy_types::*;

/// 规范化路径用于比较
pub fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

/// 检查路径是否以另一个路径为前缀
fn path_starts_with_normalized(base: &Path, prefix: &Path) -> bool {
    let normalized_base = normalize_path_for_comparison(base);
    let normalized_prefix = normalize_path_for_comparison(prefix);
    normalized_base.starts_with(&normalized_prefix)
}

/// 检查相对路径是否是安全的
pub fn is_safe_relative_path(path: &str) -> bool {
    if path == ".." {
        return false;
    }
    if path.starts_with("../") || path.starts_with("..\\") {
        return false;
    }
    true
}

/// 工具执行安全策略
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    pub autonomy: AutonomyLevel,
    pub workspace_only: bool,
    pub allowed_commands: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub allowed_roots: Vec<PathBuf>,
    pub max_actions_per_hour: u32,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
    pub tracker: ActionTracker,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            workspace_only: false,
            allowed_commands: vec![
                "git".into(),
                "npm".into(),
                "cargo".into(),
                "python3".into(),
                "python".into(),
                "R".into(),
                "Rscript".into(),
                "echo".into(),
                "date".into(),
                "ls".into(),
                "cat".into(),
                "grep".into(),
                "find".into(),
                "pwd".into(),
                "wc".into(),
                "metadata".into(),
                "tail".into(),
                "dir".into(),
                "type".into(),
                "findstr".into(),
                "where".into(),
                "cd".into(),
                "copy".into(),
                "xcopy".into(),
                "move".into(),
                "del".into(),
                "mkdir".into(),
                "rmdir".into(),
                "cls".into(),
                "ver".into(),
                "systeminfo".into(),
                "tasklist".into(),
            ],
            forbidden_paths: vec![
                "/etc".into(),
                "/root".into(),
                "/usr".into(),
                "~/.ssh".into(),
                "~/.gnupg".into(),
                "~/.aws".into(),
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
    pub async fn is_path_allowed_for_read<P: AsRef<Path>>(
        &self,
        path: P,
        workspace_dir: &Path,
    ) -> bool {
        let path = path.as_ref();
        let path_str = path.to_string_lossy();
        if !is_safe_relative_path(&path_str) {
            return false;
        }
        for forbidden in &self.forbidden_paths {
            let expanded = shellexpand::tilde(forbidden);
            if path_str.starts_with(expanded.as_ref()) {
                return false;
            }
        }
        if !self.workspace_only {
            return true;
        }
        if !path.is_absolute() {
            return true;
        }
        path_starts_with_normalized(path, workspace_dir)
            || self
                .allowed_roots
                .iter()
                .any(|r| path_starts_with_normalized(path, r))
    }

    pub async fn is_path_allowed_for_write<P: AsRef<Path>>(
        &self,
        path: P,
        workspace_dir: &Path,
    ) -> bool {
        let path = path.as_ref();
        let path_str = path.to_string_lossy();
        if !is_safe_relative_path(&path_str) {
            return false;
        }
        for forbidden in &self.forbidden_paths {
            let expanded = shellexpand::tilde(forbidden);
            if path_str.starts_with(expanded.as_ref()) {
                return false;
            }
        }
        if !self.workspace_only {
            return true;
        }
        if !path.is_absolute() {
            return true;
        }
        path_starts_with_normalized(path, workspace_dir)
            || self
                .allowed_roots
                .iter()
                .any(|r| path_starts_with_normalized(path, r))
    }

    pub fn is_command_allowed(&self, command: &str, threshold: RiskLevel) -> bool {
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }
        if command.contains('`') || command.contains("$(") {
            return false;
        }
        // 高风险阈值：放行所有命令（用户已确认承担高风险）
        if threshold == RiskLevel::High {
            return true;
        }
        let base_cmd = command.split_whitespace().next().unwrap_or("");
        let cmd_name = base_cmd.rsplit('/').next().unwrap_or(base_cmd);
        self.allowed_commands
            .iter()
            .any(|allowed| allowed == cmd_name || allowed == base_cmd)
    }

    pub fn is_rate_limited(&self) -> bool {
        self.tracker.is_at_limit(self.max_actions_per_hour)
    }

    pub fn record_action(&self) {
        self.tracker.record();
    }

    pub fn command_risk_level(&self, command: &str) -> RiskLevel {
        let command_lower = command.to_lowercase();
        let base_cmd = command_lower.split_whitespace().next().unwrap_or("");
        let high_risk = [
            "rm", "sudo", "su", "chmod", "chown", "shutdown", "reboot", "mkfs", "dd", "mount",
            "umount", "curl", "wget",
        ];
        if high_risk.contains(&base_cmd) {
            return RiskLevel::High;
        }
        let high_risk_patterns = ["rm -rf /", "rm -fr /", "mkfs", "dd if=", "sudo"];
        for pattern in &high_risk_patterns {
            if command_lower.contains(pattern) {
                return RiskLevel::High;
            }
        }
        let medium_risk = ["touch", "mkdir", "mv", "cp", "ln"];
        if medium_risk.contains(&base_cmd) {
            return RiskLevel::Medium;
        }
        if base_cmd == "git" {
            let git_ops = ["commit", "push", "reset", "clean", "rebase", "merge"];
            let second = command_lower.split_whitespace().nth(1).unwrap_or("");
            if git_ops.contains(&second) {
                return RiskLevel::Medium;
            }
        }
        RiskLevel::Low
    }

    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
        threshold: RiskLevel,
    ) -> Result<RiskLevel, String> {
        if !self.is_command_allowed(command, threshold) {
            return Err(format!("命令不在允许列表中: {command}"));
        }
        let risk = self.command_risk_level(command);
        // 高风险阈值：放行所有命令（用户已确认承担高风险）
        if threshold == RiskLevel::High {
            return Ok(risk);
        }
        match risk {
            RiskLevel::High => {
                if self.block_high_risk_commands {
                    return Err("高风险命令被策略阻止".into());
                }
                if self.autonomy == AutonomyLevel::Supervised && !approved {
                    return Err("高风险命令需要显式批准".into());
                }
            }
            RiskLevel::Medium => {
                if self.autonomy == AutonomyLevel::Supervised
                    && self.require_approval_for_medium_risk
                    && !approved
                {
                    return Err("中等风险命令需要批准".into());
                }
            }
            RiskLevel::Low => {}
        }
        Ok(risk)
    }

    /// 检查工具是否需要审批建议（基于「执行风险等级」阈值）
    ///
    /// 返回：(建议是否需要审批, 风险等级)
    ///
    /// 规则与前端「执行风险等级」保持一致：
    /// - 工具风险等级 **>** 执行风险阈值 → 需要用户审批
    /// - 工具风险等级 **≤** 执行风险阈值 → 直接执行（自动批准）
    ///
    /// `threshold` 来自 ctx[RISK_LEVEL]（per-session，由 orchestrator 从
    /// `session.metadata.risk_level` 或 `chat_send.risk_level` 写入，默认 medium）。
    pub fn check_tool_approval_needed(
        &self,
        _tool_name: &str,
        tool_risk_level: RiskLevel,
        threshold: RiskLevel,
    ) -> (bool, RiskLevel) {
        let needs_approval = tool_risk_level > threshold;
        (needs_approval, tool_risk_level)
    }

    pub fn get_tool_risk_level(
        &self,
        tool_name: &str,
        args: Option<&serde_json::Value>,
    ) -> RiskLevel {
        match tool_name {
            "read_file" | "web_fetch" | "web_search" | "glob_search" | "content_search" => {
                RiskLevel::Low
            }
            "shell" => {
                if let Some(cmd) = args.and_then(|a| a.get("command")).and_then(|c| c.as_str()) {
                    self.command_risk_level(cmd)
                } else {
                    RiskLevel::High
                }
            }
            "http_request" => RiskLevel::High,
            "write_file" | "file_edit" => RiskLevel::Medium,
            _ => RiskLevel::Medium,
        }
    }
}
