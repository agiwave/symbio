use serde::{Deserialize, Serialize};

/// Explorer configuration - Single Source of Truth
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExplorerConfig {
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default)]
    pub file_filter: Vec<String>,
}
