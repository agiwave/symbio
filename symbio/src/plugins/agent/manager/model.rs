use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 物理存储基目录 (运行时填充，不序列化)
    #[serde(skip)]
    pub base_dir: std::path::PathBuf,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            base_dir: std::path::PathBuf::new(),
        }
    }
}
