use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub status: String,
    pub model: String,
    pub api_base: String,
    pub has_api_key: bool,
}
