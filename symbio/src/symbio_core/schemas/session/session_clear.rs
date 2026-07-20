// Corresponding Frontend: tauri/src/schemas/session_clear.ts
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub session_id: String,
}

/// Session cleared message
pub type Response = String;
