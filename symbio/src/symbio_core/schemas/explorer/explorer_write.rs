// Corresponding Frontend: tauri/src/schemas/explorer_write.ts
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub path: String,
    pub content: String,
}

/// File write success message
pub type Response = String;
