use serde::{Deserialize, Serialize};

/// Memory configuration - Single Source of Truth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// 存储目录
    #[serde(default = "default_storage_dir")]
    pub storage_dir: String,

    /// 存储后端 (jsonl, dir, sqlite)
    #[serde(default = "default_storage_backend")]
    pub storage_backend: String,

    /// 存储格式 (json, yaml) - 仅对 dir 后端有效
    #[serde(default = "default_storage_format")]
    pub storage_format: String,

    /// 最大条目数
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    /// 预定义分类
    #[serde(default = "default_categories")]
    pub categories: Vec<String>,
}

fn default_storage_backend() -> String {
    "dir".to_string()
}

fn default_storage_format() -> String {
    "yaml".to_string()
}

fn default_storage_dir() -> String {
    dirs::data_local_dir()
        .map(|p| {
            p.join("symbio")
                .join("memory")
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_else(|| "~/.local/share/symbio/memory".to_string())
}

fn default_max_entries() -> usize {
    1000
}

fn default_categories() -> Vec<String> {
    vec![
        "preference".to_string(),
        "fact".to_string(),
        "instruction".to_string(),
    ]
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            storage_dir: default_storage_dir(),
            storage_backend: default_storage_backend(),
            storage_format: default_storage_format(),
            max_entries: default_max_entries(),
            categories: default_categories(),
        }
    }
}
