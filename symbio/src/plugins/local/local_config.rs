use serde::{Deserialize, Serialize};

use super::policy::{AutonomyLevel, PolicyRules};

/// 本地工具配置（`<根>/local/PLUGIN.yml`）
///
/// 策略字段的默认值即「全放开」——与 [`PolicyRules::default`] 同一口径
/// （理由见其文档）。容器级 `#[serde(default)]`：旧配置文件缺字段时逐字段
/// 回落默认值，不要求用户配置全量键。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalConfig {
    pub shell_enabled: bool,
    pub file_enabled: bool,
    pub shell_timeout: u64,
    pub autonomy: AutonomyLevel,
    pub workspace_only: bool,
    /// 命令白名单；**空 = 不限制**
    pub allowed_commands: Vec<String>,
    /// 路径黑名单（支持 `~` 展开）
    pub forbidden_paths: Vec<String>,
    /// 额外允许的读取根目录（`workspace_only` 开启时生效）
    pub allowed_roots: Vec<String>,
    /// 每小时动作上限；**0 = 不限流**
    pub max_actions_per_hour: u32,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
}

// 手写而非 derive：`AutonomyLevel` 的 derive 默认是 Supervised，与「默认全放开」相悖
impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            shell_enabled: true,
            file_enabled: true,
            shell_timeout: 60,
            autonomy: AutonomyLevel::Full,
            workspace_only: false,
            allowed_commands: Vec::new(),
            forbidden_paths: Vec::new(),
            allowed_roots: Vec::new(),
            max_actions_per_hour: 0,
            require_approval_for_medium_risk: false,
            block_high_risk_commands: false,
        }
    }
}

impl LocalConfig {
    /// 策略部分 → [`PolicyRules`]（路径字段在此做 `String → PathBuf`）
    pub fn policy_rules(&self) -> PolicyRules {
        PolicyRules {
            autonomy: self.autonomy,
            workspace_only: self.workspace_only,
            allowed_commands: self.allowed_commands.clone(),
            forbidden_paths: self.forbidden_paths.clone(),
            allowed_roots: self
                .allowed_roots
                .iter()
                .map(std::path::PathBuf::from)
                .collect(),
            max_actions_per_hour: self.max_actions_per_hour,
            require_approval_for_medium_risk: self.require_approval_for_medium_risk,
            block_high_risk_commands: self.block_high_risk_commands,
        }
    }
}
