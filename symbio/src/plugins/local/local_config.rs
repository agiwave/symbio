use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    #[serde(default = "default_true")]
    pub shell_enabled: bool,
    #[serde(default = "default_true")]
    pub file_enabled: bool,
    #[serde(default = "default_shell_timeout")]
    pub shell_timeout: u64,
}

fn default_true() -> bool {
    true
}
fn default_shell_timeout() -> u64 {
    60
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            shell_enabled: true,
            file_enabled: true,
            shell_timeout: 60,
        }
    }
}
