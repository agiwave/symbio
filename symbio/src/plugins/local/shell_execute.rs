// Corresponding Frontend: tauri/src/protocols/shell_execute.ts
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub command: String,
    pub approved: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub exit_code: Option<i32>,
    pub output: String,
    pub risk_level: String,
}
