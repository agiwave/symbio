use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SourceInfo {
    pub path: String,
    pub level: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoadResponse {
    pub content: String,
    pub sources: Vec<SourceInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListResponse {
    pub files: Vec<SourceInfo>,
}
