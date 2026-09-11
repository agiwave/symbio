// Corresponding Frontend: tauri/src/schemas/skill.ts
use serde::{Deserialize, Serialize};

/// Skill 执行响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResponse {
    pub name: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub base_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
}
